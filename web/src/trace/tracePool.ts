/**
 * Path tracing across workers: each worker loads the engine, opens the scene's bytes, and traces
 * a shard of the render (interleaved rows); the page hands their sums to the engine's collecting
 * render (`traceMerge`), whose picture is the single tracer's.
 */
import type { TraceEngine, TraceReply, TraceShard } from './protocol'

/** The most workers a render uses. */
export const MAX_TRACE_WORKERS = 8

/** Workers for a render on this machine: one per core but the page's, at most eight. */
export function autoTraceWorkers() {
  const cores = typeof navigator === 'undefined' ? 1 : navigator.hardwareConcurrency || 1
  return Math.max(1, Math.min(MAX_TRACE_WORKERS, cores - 1))
}

/** Whether this page can start trace workers at all. */
export function traceWorkersAvailable() {
  return typeof Worker !== 'undefined'
}

const compiling = new Map<string, Promise<WebAssembly.Module | null>>()

/** The engine's WebAssembly compiled once per page, for every worker to start from; `null` when it cannot be. */
function compiledEngine(engine: TraceEngine) {
  const url = new URL(`${engine.module}_bg.wasm`, engine.base.endsWith('/') ? engine.base : `${engine.base}/`).href
  let compiled = compiling.get(url)
  if (!compiled) {
    compiled = WebAssembly.compileStreaming(fetch(url)).catch(async () => {
      // A server without the WebAssembly media type: compile the bytes instead.
      try {
        return await WebAssembly.compile(await (await fetch(url)).arrayBuffer())
      } catch {
        return null
      }
    })
    compiling.set(url, compiled)
  }
  return compiled
}

/** A render's options, as `render.trace.start` takes them. */
export type TraceOptions = Omit<TraceShard, 'index' | 'count'>

/**
 * A render split across `count` workers. `merge` gets each worker's sums in the order they come
 * (one call at a time); `run` settles when every shard is done, and fails when a worker does.
 */
export class TracePool {
  private readonly workers: Worker[] = []
  private stopped = false

  constructor(private readonly count: number) {}

  async run(engine: TraceEngine, snapshot: Uint8Array, options: TraceOptions, previewMs: number, merge: (sums: Uint8Array) => Promise<void>) {
    const compiled = await compiledEngine(engine)
    if (this.stopped) return
    let queue = Promise.resolve()
    const shards = Array.from({ length: this.count }, (_, index) =>
      new Promise<void>((resolve, reject) => {
        const worker = new Worker(new URL('./traceWorker.ts', import.meta.url), { type: 'module' })
        this.workers.push(worker)
        worker.onerror = (event) => reject(new Error(event.message || 'a trace worker stopped'))
        worker.onmessage = (event: MessageEvent<TraceReply>) => {
          const reply = event.data
          if (reply.kind === 'failed') {
            reject(new Error(reply.message))
            return
          }
          // Merges run one at a time, in arrival order; a shard is done once its last merge is.
          queue = queue.then(() => (this.stopped ? undefined : merge(new Uint8Array(reply.sums))))
          if (reply.done) queue.then(resolve, reject)
          else queue.catch(reject)
        }
        const bytes = snapshot.slice().buffer as ArrayBuffer
        worker.postMessage({ engine, compiled, bytes, shard: { ...options, index, count: this.count }, previewMs }, [bytes])
      }),
    )
    await Promise.all(shards).finally(() => this.stop())
  }

  /** Stops every worker at once. */
  stop() {
    this.stopped = true
    for (const worker of this.workers.splice(0)) worker.terminate()
  }
}
