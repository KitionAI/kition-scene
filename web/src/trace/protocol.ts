/** Messages between the render dialog's trace pool and its workers. */

/** One shard of a render: every `count`-th row from row `index`, with the render's options. */
export type TraceShard = {
  index: number
  count: number
  samples: number
  denoise: boolean
  bounces?: number
  seed?: number
  frame?: number
}

/** Where the engine is: its folder (a full URL), module, and document class. */
export type TraceEngine = { base: string; module: string; documentClass: string }

/**
 * Page to worker: open the scene's bytes with the engine and trace a shard. `compiled` is the
 * engine's WebAssembly compiled once for every worker, when the page could share it.
 */
export type TraceRequest = { engine: TraceEngine; compiled: WebAssembly.Module | null; bytes: ArrayBuffer; shard: TraceShard; previewMs: number }

/** Worker to page: the shard's rows so far (`traceSums`), or why it stopped. */
export type TraceReply = { kind: 'sums'; sums: ArrayBuffer; done: boolean } | { kind: 'failed'; message: string }
