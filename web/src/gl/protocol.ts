/**
 * The GPU viewport's protocol with the engine (bindings/wasm/src/gl.rs): `glScene` describes a
 * frame as JSON, `glMesh` serves one object's geometry as a binary buffer, and `glWorld` the world
 * image as linear floats.
 */
import type { OrbitView, Vec3 } from './camera'

export type GlMaterial = {
  baseColor: [number, number, number, number]
  metallic: number
  roughness: number
  emission: Vec3
  alphaMode: 'opaque' | 'clip' | 'blend'
  alphaCutoff: number
  transmission: number
  ior: number
  baseColorTexture: string | null
  metallicRoughnessTexture: string | null
  normalTexture: string | null
  normalStrength: number
}

export type GlObject = {
  id: number
  key: string
  tangents: boolean
  world: number[]
  material: GlMaterial | null
  /** Drawn in place of the material: 1 sculpt mode's clay, 2 weight paint's colours. */
  paint: 1 | 2 | null
  /** 0 not selected, 1 selected, 2 active. */
  class: number
  /** Which peer (index into `peers`) selected it, when someone else did. */
  peer: number | null
}

export type GlLight = { kind: 'point' | 'sun' | 'spot' | 'area'; position: Vec3; direction: Vec3; power: Vec3; spotSize: number }

/** One depth picture of a light's shadow map, as the software viewport plans it. */
export type GlShadowFace = {
  /** World to clip in WebGL's convention, column-major. */
  viewProj: number[]
  /** Eye depth in front of the light: `depthRow · (point, 1)`. */
  depthRow: [number, number, number, number]
  size: number
  orthographic: boolean
  /** A texel's world size, at one metre in front of the light when perspective. */
  texel: number
}

export type GlShadow = { faces: GlShadowFace[]; centre: Vec3 | null; softness: number; taps: number }

/** Lines with a colour per end (3 floats each). */
export type GlLines = { positions: number[]; colors: number[] }

export type GlFrame = {
  supported: boolean
  reason: string | null
  revision: number
  width: number
  height: number
  view: OrbitView & { camera: number | null }
  /** Looking through a camera: its view, WebGL projection, eye, near plane, and the render frame
   * in the picture (x0, y0, x1, y1 from the top left). */
  camera: { viewMatrix: number[]; projection: number[]; eye: Vec3; perspective: boolean; near: number; frame: [number, number, number, number] } | null
  shading: 'solid' | 'wireframe' | 'material' | 'rendered'
  grid: boolean
  world: { color: Vec3; strength: number }
  /** The world image (material view): its irradiance coefficients and turn; `glWorld` has its
   * texels, fetched when the key changes. */
  environment: { key: string; coefficients: number[]; turn: number[] } | null
  lights: GlLight[]
  /** Each light's shadow map (material view), in the order of `lights`. */
  shadows: Array<GlShadow | null>
  /** Changes when the shadows would: the lights, or the casters' geometry and places. */
  shadowKey: string | null
  objects: GlObject[]
  /** Depth-tested flat triangles drawn after the outlines: custom bone shapes' faces. */
  fills: GlLines
  /** Depth-tested lines: objects without a surface, fields, and constraints. */
  gizmos: GlLines
  /** Lines over everything: bones being edited or posed, curve handles, the text caret. */
  front: GlLines
  /** Dots over everything: bone ends, curve and lattice points. */
  dots: { positions: number[]; colors: number[]; sizes: number[] }
  origins: Array<{ at: Vec3; active: boolean }>
  edit: { object: number; vertices: number[]; faces: number[] } | null
  peers: Array<{ color: Vec3 }>
  colors: { selected: Vec3; active: Vec3; wire: Vec3; outside: Vec3 }
}

/** One object's geometry, decoded (views into the engine's buffer). */
export type GlGeometry = {
  corners: number
  /** Per corner: position (3), normal (3), uv (2), face (1). */
  interleaved: Float32Array
  tangents: Float32Array | null
  /** Per corner: the paint value and the face's tint (4 floats). */
  paint: Float32Array | null
  closed: boolean
  /** The mesh's own vertex positions, and its edges as index pairs. */
  positions: Float32Array
  edges: Uint32Array
}

export const CORNER_FLOATS = 9

export function decodeGeometry(bytes: Uint8Array): GlGeometry | null {
  if (bytes.byteLength < 20 || bytes[0] !== 0x4b || bytes[1] !== 0x47 || bytes[2] !== 0x4c || bytes[3] !== 0x31) return null
  // Copied once, so the views below are aligned.
  const buffer = bytes.slice().buffer
  const header = new Uint32Array(buffer, 4, 4)
  const [corners, vertices, edgeIndices, flags] = [header[0], header[1], header[2], header[3]]
  const withTangents = (flags & 2) !== 0
  const withPaint = (flags & 4) !== 0
  let at = 20
  const take = (floats: number) => {
    const view = new Float32Array(buffer, at, floats)
    at += floats * 4
    return view
  }
  const interleaved = take(corners * CORNER_FLOATS)
  const tangents = withTangents ? take(corners * 4) : null
  const paint = withPaint ? take(corners * 4) : null
  const positions = take(vertices * 3)
  const edges = new Uint32Array(buffer, at, edgeIndices)
  return { corners, interleaved, tangents, paint, closed: (flags & 1) !== 0, positions, edges }
}

/** The world image: linear RGBA floats, top row first. */
export type GlWorld = { width: number; height: number; texels: Float32Array }

export function decodeWorld(bytes: Uint8Array): GlWorld | null {
  if (bytes.byteLength < 12 || bytes[0] !== 0x4b || bytes[1] !== 0x47 || bytes[2] !== 0x57 || bytes[3] !== 0x31) return null
  const buffer = bytes.slice().buffer
  const [width, height] = new Uint32Array(buffer, 4, 2)
  if (bytes.byteLength < 12 + width * height * 16) return null
  return { width, height, texels: new Float32Array(buffer, 12, width * height * 4) }
}
