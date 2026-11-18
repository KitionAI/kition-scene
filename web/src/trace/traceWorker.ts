/// <reference lib="webworker" />
/**
 * A path tracing worker: loads the kit's engine, opens the scene's bytes, and traces one shard
 * of the render, posting its rows' sums every so often and when they are done. The page stops
 * it by terminating it.
 */
import type { TraceReply, TraceRequest } from './protocol'

/** The worker time one step takes: short, so sums go out close to when they are due. */
const STEP_MS = 40

type ShardDocument = {
  traceShard(json: string): string
  traceStep(budgetMs: number): string
  traceSums(): Uint8Array
  free(): void
}
type EngineModule = { default: (init: { module_or_path: URL | WebAssembly.Module }) => Promise<unknown> } & Record<string, unknown>

const scope = self as unknown as DedicatedWorkerGlobalScope

function post(reply: TraceReply, transfer: Transferable[] = []) {
  scope.postMessage(reply, transfer)
}

function sendSums(doc: ShardDocument, done: boolean) {
  const sums = doc.traceSums()
  const buffer = sums.buffer.slice(sums.byteOffset, sums.byteOffset + sums.byteLength) as ArrayBuffer
  post({ kind: 'sums', sums: buffer, done }, [buffer])
}

scope.onmessage = async (event: MessageEvent<TraceRequest>) => {
  const { engine, compiled, bytes, shard, previewMs } = event.data
  try {
    const base = new URL(engine.base.endsWith('/') ? engine.base : `${engine.base}/`)
    const module = (await import(/* @vite-ignore */ new URL(`${engine.module}.js`, base).href)) as EngineModule
    await module.default({ module_or_path: compiled ?? new URL(`${engine.module}_bg.wasm`, base) })
    const documentClass = module[engine.documentClass] as { open(name: string, bytes: Uint8Array): ShardDocument } | undefined
    if (!documentClass) throw new Error(`${engine.module} has no ${engine.documentClass}`)
    const doc = documentClass.open('Scene.kscene', new Uint8Array(bytes))
    doc.traceShard(JSON.stringify(shard))
    let sent = performance.now()
    for (;;) {
      const progress = JSON.parse(doc.traceStep(STEP_MS)) as { done: boolean }
      if (progress.done) {
        sendSums(doc, true)
        break
      }
      if (performance.now() - sent >= previewMs) {
        sendSums(doc, false)
        sent = performance.now()
      }
    }
    doc.free()
  } catch (error) {
    post({ kind: 'failed', message: error instanceof Error ? error.message : String(error) })
  }
}
