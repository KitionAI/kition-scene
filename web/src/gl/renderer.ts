/**
 * The GPU viewport's WebGL2 renderer. It draws what the engine describes (`glScene`, `glMesh`)
 * the way the software viewport does (crates/render/raster/src/lib.rs): the same background,
 * studio light, material model with its shadow maps (drawn here from the engine's plans) and
 * world image, sculpt mode's clay and weight paint's colours, grid, wires, edit-mode overlays,
 * gizmos, the camera's frame, and two-pixel selection outlines from an id buffer.
 */
import { multiply, normalMatrix, projection, viewMatrix, type Mat4, type OrbitView } from './camera'
import { CORNER_FLOATS, type GlFrame, type GlGeometry, type GlObject, type GlWorld } from './protocol'

const ENCODE = /* glsl */ `
vec3 encode(vec3 c) {
  c = clamp(c, 0.0, 1.0);
  return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}
vec3 decode(vec3 c) {
  return mix(c / 12.92, pow((c + 0.055) / 1.055, vec3(2.4)), step(0.04045, c));
}`

/** The world image looked up by direction (crates/render/raster/src/environment.rs). */
const ENVIRONMENT = /* glsl */ `
uniform bool uHasEnv;
uniform highp sampler2D uEnvTex;
uniform mat3 uEnvTurn;
uniform vec3 uEnvSh[9];
vec3 envRadiance(vec3 direction) {
  vec3 d = normalize(uEnvTurn * direction);
  float u = 0.5 - atan(d.y, d.x) / 6.28318531;
  float v = 0.5 + asin(clamp(d.z, -1.0, 1.0)) / 3.14159265;
  // Rows go top first: the texture's t runs down from the zenith.
  return textureLod(uEnvTex, vec2(fract(u), 1.0 - clamp(v, 0.0, 1.0)), 0.0).rgb;
}
vec3 envDiffuse(vec3 normal) {
  vec3 n = normalize(uEnvTurn * normal);
  float x = n.x, y = n.y, z = n.z;
  const float c1 = 0.429043, c2 = 0.511664, c3 = 0.743125, c4 = 0.886227, c5 = 0.247708;
  vec3 e = uEnvSh[8] * (c1 * (x * x - y * y)) + uEnvSh[6] * (c3 * z * z) + uEnvSh[0] * c4 - uEnvSh[6] * c5
    + (uEnvSh[4] * (x * y) + uEnvSh[7] * (x * z) + uEnvSh[5] * (y * z)) * (2.0 * c1)
    + (uEnvSh[3] * x + uEnvSh[1] * y + uEnvSh[2] * z) * (2.0 * c2);
  return max(e / 3.14159265, vec3(0.0));
}`

/**
 * Shadow lookups, as the software viewport makes them (crates/render/raster/src/shadow.rs): each
 * light's row of the shadow data texture holds a header (faces, softness, taps, cube), the cube's centre
 * and whether the map is orthographic, then per face its view-projection, depth row, and tile in
 * the depth atlas (x, y, size, texel). Rows of the software map run down from the top.
 */
const SHADOWS = /* glsl */ `
uniform bool uShadows;
uniform highp sampler2D uShadowData;
uniform highp sampler2D uShadowAtlas;
float shadowVisibility(int light, vec3 p, vec3 normal, vec3 toLight) {
  vec4 head = texelFetch(uShadowData, ivec2(0, light), 0);
  int faces = int(head.x + 0.5);
  if (faces == 0) return 1.0;
  vec4 info = texelFetch(uShadowData, ivec2(1, light), 0);
  vec3 n = dot(normal, toLight) < 0.0 ? -normal : normal;
  int face = 0;
  if (head.w > 0.5) {
    vec3 o = p - info.xyz;
    vec3 a = abs(o);
    if (a.x >= a.y && a.x >= a.z) face = o.x >= 0.0 ? 0 : 1;
    else if (a.y >= a.z) face = o.y >= 0.0 ? 2 : 3;
    else face = o.z >= 0.0 ? 4 : 5;
  }
  if (face >= faces) return 1.0;
  int base = 2 + face * 6;
  mat4 vp = mat4(texelFetch(uShadowData, ivec2(base, light), 0), texelFetch(uShadowData, ivec2(base + 1, light), 0), texelFetch(uShadowData, ivec2(base + 2, light), 0), texelFetch(uShadowData, ivec2(base + 3, light), 0));
  vec4 row = texelFetch(uShadowData, ivec2(base + 4, light), 0);
  vec4 tile = texelFetch(uShadowData, ivec2(base + 5, light), 0);
  bool ortho = info.w > 0.5;
  float depth = dot(row, vec4(p, 1.0));
  if (depth <= 0.0) return 1.0;
  float texel = ortho ? tile.w : tile.w * depth;
  float cosine = clamp(dot(n, toLight), 0.05, 1.0);
  float slope = sqrt(1.0 - cosine * cosine) / cosine;
  vec3 shift = n * texel * 1.5;
  vec4 clip = vp * vec4(p + shift, 1.0);
  if (clip.w <= 0.0) return 1.0;
  vec2 ndc = clip.xy / clip.w;
  if (abs(ndc.x) > 1.0 || abs(ndc.y) > 1.0) return 1.0;
  float size = tile.z;
  float sx = (ndc.x + 1.0) * 0.5 * size;
  float sy = (1.0 - ndc.y) * 0.5 * size;
  float reference = depth + dot(row.xyz, shift) - texel * (1.0 + 2.0 * min(slope, 8.0));
  float spread = clamp(ortho ? head.y : head.y / max(depth, 0.1), 0.75, 6.0);
  int taps = int(head.z + 0.5);
  float middle = (float(taps) - 1.0) / 2.0;
  int isize = int(size + 0.5);
  int lit = 0;
  for (int j = 0; j < 6; j++) {
    if (j >= taps) break;
    int y = int(sy + (float(j) - middle) * spread);
    if (y < 0 || y >= isize) {
      lit += taps;
      continue;
    }
    for (int i = 0; i < 6; i++) {
      if (i >= taps) break;
      int x = int(sx + (float(i) - middle) * spread);
      if (x < 0 || x >= isize || reference <= texelFetch(uShadowAtlas, ivec2(int(tile.x) + x, int(tile.y) + isize - 1 - y), 0).r) lit++;
    }
  }
  return float(lit) / float(taps * taps);
}`

const MESH_VS = /* glsl */ `#version 300 es
layout(location = 0) in vec3 aPos;
layout(location = 1) in vec3 aNormal;
layout(location = 2) in vec2 aUv;
layout(location = 3) in float aFace;
layout(location = 4) in vec4 aTangent;
layout(location = 5) in vec4 aPaint;
uniform mat4 uModel;
uniform mat4 uViewProj;
uniform mat3 uNormal;
out vec3 vWorld;
out vec3 vNormal;
out vec2 vUv;
flat out int vFace;
out vec4 vTangent;
out vec4 vPaint;
void main() {
  vPaint = aPaint;
  vec4 world = uModel * vec4(aPos, 1.0);
  vWorld = world.xyz;
  vNormal = uNormal * aNormal;
  vUv = aUv;
  vFace = int(aFace + 0.5);
  vTangent = vec4(mat3(uModel) * aTangent.xyz, aTangent.w);
  gl_Position = uViewProj * world;
}`

const MESH_FS = /* glsl */ `#version 300 es
precision highp float;
precision highp int;
in vec3 vWorld;
in vec3 vNormal;
in vec2 vUv;
flat in int vFace;
in vec4 vTangent;
in vec4 vPaint;
uniform int uShading; // 0 solid, 1 material
uniform int uPaint; // 0 none, 1 sculpt mode's clay, 2 weight paint
uniform mat4 uView;
uniform bool uPerspective;
uniform vec3 uEye;
uniform vec4 uBase;
uniform float uMetallic;
uniform float uRoughness;
uniform vec3 uEmission;
uniform int uAlphaMode; // 0 opaque, 1 clip, 2 blend
uniform float uCutoff;
uniform float uTransmission;
uniform float uIor;
uniform bool uHasBaseTex;
uniform sampler2D uBaseTex;
uniform bool uHasMrTex;
uniform sampler2D uMrTex;
uniform bool uHasNormalTex;
uniform sampler2D uNormalTex;
uniform float uNormalStrength;
uniform int uLightCount;
uniform int uLightKind[8];
uniform vec3 uLightPos[8];
uniform vec3 uLightDir[8];
uniform vec3 uLightPower[8];
uniform float uLightSpot[8];
uniform vec3 uAmbient;
uniform float uClass;
uniform bool uEditFaces;
uniform highp sampler2D uFaceSel;
uniform vec3 uSelColor;
layout(location = 0) out vec4 oColor;
layout(location = 1) out vec4 oClass;
${ENCODE}
${ENVIRONMENT}
${SHADOWS}

/** Weight paint's ramp: blue, cyan, green, yellow, red. */
vec3 ramp(float weight) {
  float w = clamp(weight, 0.0, 1.0);
  if (w <= 0.25) return mix(vec3(0.0, 0.0, 1.0), vec3(0.0, 1.0, 1.0), w / 0.25);
  if (w <= 0.5) return mix(vec3(0.0, 1.0, 1.0), vec3(0.0, 1.0, 0.0), (w - 0.25) / 0.25);
  if (w <= 0.75) return mix(vec3(0.0, 1.0, 0.0), vec3(1.0, 1.0, 0.0), (w - 0.5) / 0.25);
  return mix(vec3(1.0, 1.0, 0.0), vec3(1.0, 0.0, 0.0), (w - 0.75) / 0.25);
}

vec3 studio(vec3 normal, vec3 base) {
  vec3 n = normalize(mat3(uView) * normal);
  vec3 toward = uPerspective ? -(uView * vec4(vWorld, 1.0)).xyz : vec3(0.0, 0.0, 1.0);
  if (dot(n, toward) < 0.0) n = -n;
  float key = max(dot(n, normalize(vec3(-0.35, 0.55, 0.76))), 0.0);
  float fill = max(dot(n, normalize(vec3(0.6, -0.2, 0.4))), 0.0);
  return base * (0.22 + 0.68 * key + 0.18 * fill);
}

void main() {
  vec3 normal = normalize(vNormal);
  vec3 surface = normal;
  vec3 lit;
  float alpha = 1.0;
  if (uPaint == 1) {
    // Clay grey tinted by the face set, darker where masked.
    lit = studio(normal, 0.6 * vPaint.yzw) * (1.0 - 0.65 * clamp(vPaint.x, 0.0, 1.0));
  } else if (uPaint == 2) {
    lit = ramp(vPaint.x) * (0.35 + 0.65 * studio(normal, vec3(1.0)).r);
  } else if (uShading == 0) {
    lit = studio(normal, vec3(0.6));
  } else {
    vec3 base = uBase.rgb;
    alpha = uBase.a;
    float metallic = uMetallic;
    float roughness = uRoughness;
    if (uHasBaseTex) {
      vec4 sampled = texture(uBaseTex, vUv);
      base *= sampled.rgb;
      alpha *= sampled.a;
    }
    if (uHasMrTex) {
      vec4 sampled = texture(uMrTex, vUv);
      roughness *= sampled.g;
      metallic *= sampled.b;
    }
    if (uHasNormalTex && dot(vTangent.xyz, vTangent.xyz) > 0.0) {
      vec3 t = normalize(vTangent.xyz - normal * dot(normal, vTangent.xyz));
      vec3 b = cross(normal, t) * vTangent.w;
      vec3 m = texture(uNormalTex, vUv).xyz * 2.0 - 1.0;
      m.xy *= uNormalStrength;
      normal = normalize(t * m.x + b * m.y + normal * m.z);
    }
    if (uAlphaMode == 1 && alpha < uCutoff) discard;
    vec3 toEye = normalize(uEye - vWorld);
    float coverage = uAlphaMode == 2 ? clamp(alpha, 0.0, 1.0) : 1.0;
    if (uTransmission > 0.0) {
      float cosine = abs(dot(normal, toEye));
      float f0 = pow((uIor - 1.0) / (uIor + 1.0), 2.0);
      float fresnel = f0 + (1.0 - f0) * pow(1.0 - cosine, 5.0);
      coverage *= 1.0 - uTransmission * (1.0 - fresnel) * 0.9;
    }
    alpha = coverage;
    if (uLightCount == 0 && !uHasEnv) {
      lit = studio(normal, base) * 1.4;
    } else {
      vec3 n = dot(normal, toEye) < 0.0 ? -normal : normal;
      vec3 diffuse = base * (1.0 - metallic);
      vec3 specular = mix(vec3(0.04), base, metallic);
      float r = clamp(roughness, 0.03, 1.0);
      float shininess = 2.0 / (r * r * r * r) - 2.0;
      if (uHasEnv) {
        // The world image: diffuse light from all around, and the world seen in the surface
        // (sharper the smoother it is).
        vec3 reflected = reflect(-toEye, n);
        float gloss = pow(1.0 - clamp(roughness, 0.0, 1.0), 2.0);
        vec3 specularLight = mix(envDiffuse(n), envRadiance(reflected), gloss);
        float facing = max(dot(n, toEye), 0.0);
        vec3 fresnel = specular + (vec3(1.0) - specular) * pow(1.0 - facing, 5.0) * gloss;
        lit = diffuse * envDiffuse(n) + fresnel * specularLight;
      } else {
        // A plain world colour: the same light from every direction, diffuse and reflected.
        float gloss = pow(1.0 - clamp(roughness, 0.0, 1.0), 2.0);
        float facing = max(dot(n, toEye), 0.0);
        vec3 fresnel = specular + (vec3(1.0) - specular) * pow(1.0 - facing, 5.0) * gloss;
        lit = (diffuse + fresnel) * uAmbient;
      }
      for (int index = 0; index < 8; index++) {
        if (index >= uLightCount) break;
        vec3 toLight;
        vec3 irradiance;
        if (uLightKind[index] == 1) {
          toLight = -uLightDir[index];
          irradiance = uLightPower[index] / 3.14159265;
        } else {
          vec3 offset = uLightPos[index] - vWorld;
          float d2 = max(dot(offset, offset), 1e-4);
          toLight = offset / sqrt(d2);
          float strength = 1.0 / (4.0 * 3.14159265 * 3.14159265 * d2);
          if (uLightKind[index] == 2) {
            float cosine = dot(-toLight, uLightDir[index]);
            float edge = cos(uLightSpot[index] / 2.0);
            strength *= clamp((cosine - edge) / max(1.0 - edge, 1e-3), 0.0, 1.0);
          }
          if (uLightKind[index] == 3) strength *= max(dot(-toLight, uLightDir[index]), 0.0);
          irradiance = uLightPower[index] * strength;
        }
        float lambert = max(dot(n, toLight), 0.0);
        if (lambert <= 0.0) continue;
        vec3 half_ = normalize(toLight + toEye);
        float gloss = pow(max(dot(n, half_), 0.0), shininess) * (shininess + 8.0) / (8.0 * 3.14159265);
        float visible = uShadows ? shadowVisibility(index, vWorld, surface, toLight) : 1.0;
        lit += (diffuse + specular * gloss) * irradiance * lambert * visible;
      }
      lit += uEmission;
    }
  }
  if (uEditFaces) {
    int width = textureSize(uFaceSel, 0).x;
    if (texelFetch(uFaceSel, ivec2(vFace % width, vFace / width), 0).r > 0.5) lit = mix(lit, uSelColor, 0.35);
  }
  oColor = vec4(encode(lit), alpha);
  oClass = vec4(uClass, 0.0, 0.0, 1.0);
}`

const LINE_VS = /* glsl */ `#version 300 es
layout(location = 0) in vec3 aPos;
layout(location = 1) in vec4 aColor;
layout(location = 2) in float aSize;
uniform mat4 uViewProj;
uniform float uBias;
// The software viewport lets a line through where it is within 0.2 % of the eye depth drawn:
// the projection's depth terms (z = a·z_eye + b) and whether it is a perspective one.
uniform vec3 uSlack;
out vec4 vColor;
void main() {
  gl_Position = uViewProj * vec4(aPos, 1.0);
  gl_Position.z -= uBias * gl_Position.w;
  if (uSlack.z > 0.5) gl_Position.z -= 0.002 * abs(uSlack.y);
  else if (uSlack.x != 0.0) gl_Position.z -= 0.002 * abs(uSlack.x) * max(-(gl_Position.z - uSlack.y) / uSlack.x, 0.0);
  gl_PointSize = aSize;
  vColor = aColor;
}`

const LINE_FS = /* glsl */ `#version 300 es
precision highp float;
in vec4 vColor;
out vec4 oColor;
${ENCODE}
void main() {
  oColor = vec4(encode(vColor.rgb), vColor.a);
}`

const QUAD_VS = /* glsl */ `#version 300 es
const vec2 corners[3] = vec2[3](vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
void main() {
  gl_Position = vec4(corners[gl_VertexID], 0.0, 1.0);
}`

const BACKGROUND_FS = /* glsl */ `#version 300 es
precision highp float;
uniform vec3 uTop;
uniform vec3 uBottom;
uniform vec2 uSize;
uniform mat4 uInverse;
out vec4 oColor;
${ENCODE}
${ENVIRONMENT}
void main() {
  if (uHasEnv) {
    // The world's radiance along each pixel's ray.
    vec2 ndc = gl_FragCoord.xy / uSize * 2.0 - 1.0;
    vec4 near = uInverse * vec4(ndc, -1.0, 1.0);
    vec4 far = uInverse * vec4(ndc, 0.0, 1.0);
    oColor = vec4(encode(envRadiance(normalize(far.xyz / far.w - near.xyz / near.w))), 1.0);
    return;
  }
  float t = 1.0 - gl_FragCoord.y / uSize.y;
  oColor = vec4(encode(mix(uTop, uBottom, t)), 1.0);
}`

/** A shadow map's depth picture: eye depth in front of the light. */
const DEPTH_VS = /* glsl */ `#version 300 es
layout(location = 0) in vec3 aPos;
uniform mat4 uModel;
uniform mat4 uViewProj;
out vec3 vWorld;
void main() {
  vec4 world = uModel * vec4(aPos, 1.0);
  vWorld = world.xyz;
  gl_Position = uViewProj * world;
}`

const DEPTH_FS = /* glsl */ `#version 300 es
precision highp float;
in vec3 vWorld;
uniform vec4 uDepthRow;
out vec4 oDepth;
void main() {
  oDepth = vec4(dot(uDepthRow, vec4(vWorld, 1.0)));
}`

/** The colour picture with outlines: selected (and active) around this person's selection, and
 * each peer's colour around theirs, two pixels wide outside the silhouette. */
const OUTLINE_FS = /* glsl */ `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D uColor;
uniform sampler2D uClass;
uniform bool uSelection;
uniform vec3 uSelected;
uniform vec3 uActive;
uniform vec3 uPeers[8];
uniform bool uDim;
uniform vec4 uFrame;
uniform vec3 uOutside;
uniform float uHeight;
out vec4 oColor;
${ENCODE}
int classAt(ivec2 at) {
  ivec2 size = textureSize(uClass, 0);
  if (at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y) return 0;
  return int(texelFetch(uClass, at, 0).r * 255.0 + 0.5);
}
void outlined();
void main() {
  outlined();
  // Looking through a camera: what is outside the render frame is dimmed.
  vec2 p = vec2(gl_FragCoord.x, uHeight - gl_FragCoord.y);
  if (uDim && (p.x < uFrame.x || p.x > uFrame.z || p.y < uFrame.y || p.y > uFrame.w)) oColor.rgb = encode(mix(decode(oColor.rgb), uOutside, 0.5));
}
void outlined() {
  ivec2 at = ivec2(gl_FragCoord.xy);
  oColor = texelFetch(uColor, at, 0);
  int here = classAt(at);
  if (here != 0) return;
  int best = 0;
  for (int dy = -2; dy <= 2; dy++) {
    for (int dx = -2; dx <= 2; dx++) {
      if (dx * dx + dy * dy > 5) continue;
      int other = classAt(at + ivec2(dx, dy));
      if (other == 0) continue;
      // The active outline wins, then this person's selection, then peers.
      int rank = other == 2 ? 3 : other == 1 ? 2 : 1;
      int bestRank = best == 2 ? 3 : best == 1 ? 2 : best >= 3 ? 1 : 0;
      if (rank > bestRank) best = other;
    }
  }
  if (best == 2 && uSelection) oColor = vec4(encode(uActive), 1.0);
  else if (best == 1 && uSelection) oColor = vec4(encode(uSelected), 1.0);
  else if (best >= 3) oColor = vec4(encode(uPeers[min(best - 3, 7)]), 1.0);
}`

const AXIS_X = srgb(0xff3352)
const AXIS_Y = srgb(0x8bdc00)
const EDIT_VERTEX: [number, number, number] = [0, 0, 0]

function srgb(rgb: number): [number, number, number] {
  return [(rgb >> 16) & 255, (rgb >> 8) & 255, rgb & 255].map((byte) => {
    const value = byte / 255
    return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4
  }) as [number, number, number]
}

type Program = { program: WebGLProgram; uniforms: Map<string, WebGLUniformLocation | null> }

type DrawnMesh = {
  key: string
  corners: number
  closed: boolean
  vao: WebGLVertexArrayObject
  buffers: WebGLBuffer[]
  /** The mesh's own vertices and edges, for wires and edit mode. */
  positions: Float32Array
  edges: Uint32Array
  edgeVao: WebGLVertexArrayObject
  edgeBuffers: WebGLBuffer[]
}

/** What a draw needs beyond the frame: the view the editor shows now, the picture size, and
 * whether it looks through the frame's camera (no gesture moved the view away from it). */
export type GlView = { view: OrbitView; width: number; height: number; camera?: boolean }

/** The shadow maps' depth pictures, packed into one float texture in cells of this many texels. */
const SHADOW_CELL = 512
/** Rows of cells in the depth atlas (columns grow with the pictures). */
const SHADOW_ROWS = 4
/** Texels per light in the shadow data texture: a header, the centre, and six faces of six. */
const SHADOW_DATA_WIDTH = 2 + 6 * 6

export class GlRenderer {
  readonly gl: WebGL2RenderingContext
  private readonly mesh: Program
  private readonly line: Program
  private readonly background: Program
  private readonly outline: Program
  private readonly depth: Program
  private readonly meshes = new Map<number, DrawnMesh>()
  private readonly textures = new Map<string, WebGLTexture | null>()
  private readonly emptyVao: WebGLVertexArrayObject
  private readonly scratchVao: WebGLVertexArrayObject
  private readonly scratch: WebGLBuffer[]
  private targets: { width: number; height: number; scene: WebGLFramebuffer; color: WebGLTexture; klass: WebGLTexture; depth: WebGLRenderbuffer; final: WebGLFramebuffer; finalColor: WebGLRenderbuffer } | null = null
  private faceSelection: { texture: WebGLTexture; object: number; faces: string } | null = null
  /** The world image, with the key it was made from. */
  private environment: { key: string; texture: WebGLTexture } | null = null
  /** The shadow maps: the depth atlas, the data each lookup needs, and the key drawn. */
  private shadows: { key: string; atlas: WebGLTexture; framebuffer: WebGLFramebuffer; depth: WebGLRenderbuffer; width: number; height: number; data: WebGLTexture } | null = null
  /** Float targets for shadow maps (EXT_color_buffer_float), when the GPU has them. */
  readonly shadowsAvailable: boolean
  /** Linear filtering of 32-bit float textures, for the world image. */
  private readonly floatLinear: boolean
  /** A 1×1 texture bound where a sampler has nothing to read. */
  private readonly blank: WebGLTexture
  private readonly blankFloat: WebGLTexture
  /** The eye, its forward direction, and the near distance, for clipping lines (perspective). */
  private clip: { eye: [number, number, number]; forward: [number, number, number]; near: number } | null = null
  /** The projection's depth terms for depth-tested lines' slack (see LINE_VS). */
  private slack: [number, number, number] = [0, 0, 0]
  /** Asked for when a texture's image is not here yet. */
  onTextureNeeded: (hash: string, data: boolean) => void = () => undefined

  constructor(readonly canvas: HTMLCanvasElement) {
    const gl = canvas.getContext('webgl2', { antialias: false, alpha: false, depth: true, stencil: false, preserveDrawingBuffer: true, premultipliedAlpha: false })
    if (!gl) throw new Error('WebGL2 is not available')
    this.gl = gl
    this.mesh = this.program(MESH_VS, MESH_FS)
    this.line = this.program(LINE_VS, LINE_FS)
    this.background = this.program(QUAD_VS, BACKGROUND_FS)
    this.outline = this.program(QUAD_VS, OUTLINE_FS)
    this.depth = this.program(DEPTH_VS, DEPTH_FS)
    this.shadowsAvailable = gl.getExtension('EXT_color_buffer_float') != null
    this.floatLinear = gl.getExtension('OES_texture_float_linear') != null
    this.blank = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, this.blank)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array([0, 0, 0, 255]))
    this.blankFloat = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, this.blankFloat)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, 1, 1, 0, gl.RGBA, gl.FLOAT, new Float32Array(4))
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
    this.emptyVao = gl.createVertexArray()!
    this.scratchVao = gl.createVertexArray()!
    this.scratch = [gl.createBuffer()!, gl.createBuffer()!, gl.createBuffer()!]
  }

  private program(vs: string, fs: string): Program {
    const gl = this.gl
    const compile = (type: number, source: string) => {
      const shader = gl.createShader(type)!
      gl.shaderSource(shader, source)
      gl.compileShader(shader)
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS) && !gl.isContextLost()) throw new Error(gl.getShaderInfoLog(shader) ?? 'shader')
      return shader
    }
    const program = gl.createProgram()!
    gl.attachShader(program, compile(gl.VERTEX_SHADER, vs))
    gl.attachShader(program, compile(gl.FRAGMENT_SHADER, fs))
    gl.linkProgram(program)
    if (!gl.getProgramParameter(program, gl.LINK_STATUS) && !gl.isContextLost()) throw new Error(gl.getProgramInfoLog(program) ?? 'program')
    return { program, uniforms: new Map() }
  }

  private uniform(program: Program, name: string) {
    let location = program.uniforms.get(name)
    if (location === undefined) {
      location = this.gl.getUniformLocation(program.program, name)
      program.uniforms.set(name, location)
    }
    return location
  }

  /** The keys of the geometry held, by object id. */
  heldKeys() {
    return new Map([...this.meshes].map(([id, mesh]) => [id, mesh.key]))
  }

  /** Drops geometry of objects no longer drawn. */
  retain(ids: ReadonlySet<number>) {
    for (const [id, mesh] of this.meshes) {
      if (!ids.has(id)) {
        this.free(mesh)
        this.meshes.delete(id)
      }
    }
  }

  private free(mesh: DrawnMesh) {
    const gl = this.gl
    gl.deleteVertexArray(mesh.vao)
    gl.deleteVertexArray(mesh.edgeVao)
    for (const buffer of [...mesh.buffers, ...mesh.edgeBuffers]) gl.deleteBuffer(buffer)
  }

  /** Uploads one object's geometry. */
  setGeometry(id: number, key: string, geometry: GlGeometry) {
    const gl = this.gl
    const old = this.meshes.get(id)
    if (old) this.free(old)
    const vao = gl.createVertexArray()!
    gl.bindVertexArray(vao)
    const interleaved = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, interleaved)
    gl.bufferData(gl.ARRAY_BUFFER, geometry.interleaved, gl.STATIC_DRAW)
    const stride = CORNER_FLOATS * 4
    for (const [location, size, offset] of [[0, 3, 0], [1, 3, 3], [2, 2, 6], [3, 1, 8]] as const) {
      gl.enableVertexAttribArray(location)
      gl.vertexAttribPointer(location, size, gl.FLOAT, false, stride, offset * 4)
    }
    const buffers = [interleaved]
    if (geometry.tangents) {
      const tangents = gl.createBuffer()!
      gl.bindBuffer(gl.ARRAY_BUFFER, tangents)
      gl.bufferData(gl.ARRAY_BUFFER, geometry.tangents, gl.STATIC_DRAW)
      gl.enableVertexAttribArray(4)
      gl.vertexAttribPointer(4, 4, gl.FLOAT, false, 0, 0)
      buffers.push(tangents)
    } else {
      gl.disableVertexAttribArray(4)
      gl.vertexAttrib4f(4, 0, 0, 0, 1)
    }
    if (geometry.paint) {
      const paint = gl.createBuffer()!
      gl.bindBuffer(gl.ARRAY_BUFFER, paint)
      gl.bufferData(gl.ARRAY_BUFFER, geometry.paint, gl.STATIC_DRAW)
      gl.enableVertexAttribArray(5)
      gl.vertexAttribPointer(5, 4, gl.FLOAT, false, 0, 0)
      buffers.push(paint)
    } else {
      gl.disableVertexAttribArray(5)
      gl.vertexAttrib4f(5, 0, 1, 1, 1)
    }
    const edgeVao = gl.createVertexArray()!
    gl.bindVertexArray(edgeVao)
    const positions = gl.createBuffer()!
    gl.bindBuffer(gl.ARRAY_BUFFER, positions)
    gl.bufferData(gl.ARRAY_BUFFER, geometry.positions, gl.STATIC_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0)
    const edges = gl.createBuffer()!
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, edges)
    gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, geometry.edges, gl.STATIC_DRAW)
    gl.bindVertexArray(null)
    this.meshes.set(id, {
      key,
      corners: geometry.corners,
      closed: geometry.closed,
      vao,
      buffers,
      positions: geometry.positions,
      edges: geometry.edges,
      edgeVao,
      edgeBuffers: [positions, edges],
    })
  }

  /** A material image, decoded (null: it cannot be shown; the material draws without it). */
  setTexture(hash: string, image: ImageBitmap | null) {
    const gl = this.gl
    if (!image) {
      this.textures.set(hash, null)
      return
    }
    const texture = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.SRGB8_ALPHA8, gl.RGBA, gl.UNSIGNED_BYTE, image)
    gl.generateMipmap(gl.TEXTURE_2D)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT)
    this.textures.set(hash, texture)
  }

  /** A data texture (normal and roughness maps are not colours: linear, not sRGB). */
  setDataTexture(hash: string, image: ImageBitmap | null) {
    const gl = this.gl
    if (!image) {
      this.textures.set(`data:${hash}`, null)
      return
    }
    const texture = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, gl.RGBA, gl.UNSIGNED_BYTE, image)
    gl.generateMipmap(gl.TEXTURE_2D)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.REPEAT)
    this.textures.set(`data:${hash}`, texture)
  }

  /** The key of the world image held, if any. */
  environmentKey() {
    return this.environment?.key ?? null
  }

  /** The world image as linear floats (null: none). */
  setEnvironment(key: string, world: GlWorld | null) {
    const gl = this.gl
    if (this.environment) gl.deleteTexture(this.environment.texture)
    this.environment = null
    if (!world) return
    const texture = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 4)
    if (this.floatLinear) {
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, world.width, world.height, 0, gl.RGBA, gl.FLOAT, world.texels)
    } else {
      // Half floats filter everywhere; radiance past their range is clipped.
      const clipped = world.texels.map((value) => Math.min(value, 65000))
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, world.width, world.height, 0, gl.RGBA, gl.FLOAT, clipped)
    }
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.REPEAT)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE)
    this.environment = { key, texture }
  }

  /** Binds the world image's uniforms on `program` (texture unit 6). */
  private bindEnvironment(program: Program, frame: GlFrame) {
    const gl = this.gl
    const environment = frame.shading === 'material' && frame.environment && this.environment?.key === frame.environment.key ? frame.environment : null
    gl.activeTexture(gl.TEXTURE6)
    gl.bindTexture(gl.TEXTURE_2D, environment ? this.environment!.texture : this.blankFloat)
    gl.uniform1i(this.uniform(program, 'uEnvTex'), 6)
    gl.uniform1i(this.uniform(program, 'uHasEnv'), environment ? 1 : 0)
    if (environment) {
      gl.uniformMatrix3fv(this.uniform(program, 'uEnvTurn'), false, new Float32Array(environment.turn))
      gl.uniform3fv(this.uniform(program, 'uEnvSh'), new Float32Array(environment.coefficients))
    }
  }

  /** Whether the frame wants shadows the GPU cannot draw here. */
  shadowsMissing(frame: GlFrame) {
    return frame.shading === 'material' && frame.shadows.some((shadow) => shadow != null) && !this.shadowsAvailable
  }

  /**
   * Draws the frame's shadow maps into the depth atlas and their lookup data, when their key
   * changed (the view moving keeps them). Every caster's geometry must be the frame's.
   */
  private drawShadows(frame: GlFrame, drawn: Array<{ object: GlObject; mesh: DrawnMesh }>) {
    const gl = this.gl
    if (frame.shading !== 'material' || !frame.shadowKey || !this.shadowsAvailable || !frame.shadows.some((shadow) => shadow != null)) return false
    if (this.shadows?.key === frame.shadowKey) return true
    if (drawn.length !== frame.objects.length) return false
    // Tiles: whole cells per face, the larger faces first, each in the first free square.
    const faces = frame.shadows.flatMap((shadow, light) => (shadow ? shadow.faces.map((face, index) => ({ light, index, face, cells: Math.max(1, Math.ceil(face.size / SHADOW_CELL)) })) : []))
    const area = faces.reduce((sum, face) => sum + face.cells * face.cells, 0)
    let columns = SHADOW_ROWS * Math.max(1, Math.ceil(area / (SHADOW_ROWS * SHADOW_ROWS)))
    const place = () => {
      const used = new Uint8Array(columns * SHADOW_ROWS)
      const tiles = new Map<(typeof faces)[number], [number, number]>()
      for (const face of [...faces].sort((a, b) => b.cells - a.cells)) {
        let found: [number, number] | null = null
        for (let row = 0; row + face.cells <= SHADOW_ROWS && !found; row += face.cells) {
          for (let column = 0; column + face.cells <= columns && !found; column += face.cells) {
            let free = true
            for (let dy = 0; dy < face.cells && free; dy += 1) for (let dx = 0; dx < face.cells && free; dx += 1) free = used[(row + dy) * columns + column + dx] === 0
            if (free) found = [column, row]
          }
        }
        if (!found) return null
        for (let dy = 0; dy < face.cells; dy += 1) for (let dx = 0; dx < face.cells; dx += 1) used[(found[1] + dy) * columns + found[0] + dx] = 1
        tiles.set(face, found)
      }
      return tiles
    }
    let tiles = place()
    while (!tiles) {
      columns += SHADOW_ROWS
      tiles = place()
    }
    const [width, height] = [columns * SHADOW_CELL, SHADOW_ROWS * SHADOW_CELL]
    if (width > gl.getParameter(gl.MAX_TEXTURE_SIZE)) return false
    if (!this.shadows || this.shadows.width !== width || this.shadows.height !== height) {
      if (this.shadows) {
        gl.deleteTexture(this.shadows.atlas)
        gl.deleteTexture(this.shadows.data)
        gl.deleteFramebuffer(this.shadows.framebuffer)
        gl.deleteRenderbuffer(this.shadows.depth)
      }
      const atlas = gl.createTexture()!
      gl.bindTexture(gl.TEXTURE_2D, atlas)
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.R32F, width, height)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
      const depth = gl.createRenderbuffer()!
      gl.bindRenderbuffer(gl.RENDERBUFFER, depth)
      gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, width, height)
      const framebuffer = gl.createFramebuffer()!
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer)
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, atlas, 0)
      gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth)
      const data = gl.createTexture()!
      gl.bindTexture(gl.TEXTURE_2D, data)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
      this.shadows = { key: '', atlas, framebuffer, depth, width, height, data }
    }
    const shadows = this.shadows
    // The lookup data, a row per light.
    const lights = frame.lights.length
    const data = new Float32Array(SHADOW_DATA_WIDTH * Math.max(1, lights) * 4)
    frame.shadows.forEach((shadow, light) => {
      if (!shadow || light >= lights) return
      const row = light * SHADOW_DATA_WIDTH * 4
      data.set([shadow.faces.length, shadow.softness, shadow.taps, shadow.centre ? 1 : 0], row)
      data.set([...(shadow.centre ?? [0, 0, 0]), shadow.faces[0]?.orthographic ? 1 : 0], row + 4)
    })
    for (const face of faces) {
      const [column, row] = tiles.get(face)!
      const at = face.light * SHADOW_DATA_WIDTH * 4 + (2 + face.index * 6) * 4
      data.set(face.face.viewProj, at)
      data.set(face.face.depthRow, at + 16)
      data.set([column * SHADOW_CELL, row * SHADOW_CELL, face.face.size, face.face.texel], at + 20)
    }
    gl.bindTexture(gl.TEXTURE_2D, shadows.data)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA32F, SHADOW_DATA_WIDTH, Math.max(1, lights), 0, gl.RGBA, gl.FLOAT, data)
    // The depth pictures. A closed mesh draws only its faces turned away from the light, so its
    // lit side never shadows itself; open meshes draw both sides.
    gl.bindFramebuffer(gl.FRAMEBUFFER, shadows.framebuffer)
    gl.drawBuffers([gl.COLOR_ATTACHMENT0])
    gl.useProgram(this.depth.program)
    gl.enable(gl.DEPTH_TEST)
    gl.depthFunc(gl.LESS)
    gl.depthMask(true)
    gl.disable(gl.BLEND)
    gl.enable(gl.SCISSOR_TEST)
    for (const face of faces) {
      const [column, row] = tiles.get(face)!
      const size = face.face.size
      gl.viewport(column * SHADOW_CELL, row * SHADOW_CELL, size, size)
      gl.scissor(column * SHADOW_CELL, row * SHADOW_CELL, size, size)
      gl.clearBufferfv(gl.COLOR, 0, [1e30, 1e30, 1e30, 1e30])
      gl.clearBufferfv(gl.DEPTH, 0, [1])
      gl.uniformMatrix4fv(this.uniform(this.depth, 'uViewProj'), false, new Float32Array(face.face.viewProj))
      gl.uniform4fv(this.uniform(this.depth, 'uDepthRow'), face.face.depthRow)
      for (const { object, mesh } of drawn) {
        if (!mesh.corners) continue
        const world = new Float32Array(object.world)
        gl.uniformMatrix4fv(this.uniform(this.depth, 'uModel'), false, world)
        if (mesh.closed) {
          gl.enable(gl.CULL_FACE)
          gl.cullFace(gl.FRONT)
          gl.frontFace(determinant(world) < 0 ? gl.CW : gl.CCW)
        } else gl.disable(gl.CULL_FACE)
        gl.bindVertexArray(mesh.vao)
        gl.drawArrays(gl.TRIANGLES, 0, mesh.corners)
      }
    }
    gl.bindVertexArray(null)
    gl.disable(gl.SCISSOR_TEST)
    gl.disable(gl.CULL_FACE)
    gl.cullFace(gl.BACK)
    shadows.key = frame.shadowKey
    return true
  }

  private texture(key: string, hash: string) {
    if (!this.textures.has(key)) {
      this.textures.set(key, null)
      this.onTextureNeeded(hash, key.startsWith('data:'))
    }
    return this.textures.get(key) ?? null
  }

  private ensureTargets(width: number, height: number) {
    const gl = this.gl
    if (this.targets && this.targets.width === width && this.targets.height === height) return this.targets
    if (this.targets) {
      gl.deleteFramebuffer(this.targets.scene)
      gl.deleteFramebuffer(this.targets.final)
      gl.deleteTexture(this.targets.color)
      gl.deleteTexture(this.targets.klass)
      gl.deleteRenderbuffer(this.targets.depth)
      gl.deleteRenderbuffer(this.targets.finalColor)
    }
    const colorTexture = () => {
      const texture = gl.createTexture()!
      gl.bindTexture(gl.TEXTURE_2D, texture)
      gl.texStorage2D(gl.TEXTURE_2D, 1, gl.RGBA8, width, height)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
      return texture
    }
    const color = colorTexture()
    const klass = colorTexture()
    const depth = gl.createRenderbuffer()!
    gl.bindRenderbuffer(gl.RENDERBUFFER, depth)
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT24, width, height)
    const scene = gl.createFramebuffer()!
    gl.bindFramebuffer(gl.FRAMEBUFFER, scene)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, color, 0)
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT1, gl.TEXTURE_2D, klass, 0)
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth)
    const finalColor = gl.createRenderbuffer()!
    gl.bindRenderbuffer(gl.RENDERBUFFER, finalColor)
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, width, height)
    const final = gl.createFramebuffer()!
    gl.bindFramebuffer(gl.FRAMEBUFFER, final)
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, finalColor)
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depth)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
    this.targets = { width, height, scene, color, klass, depth, final, finalColor }
    return this.targets
  }

  /** Lines (and points) from flat arrays: positions (3 each), colours (4 each), sizes. */
  private drawLines(mode: number, viewProj: Mat4, positions: Float32Array, colors: Float32Array, sizes: Float32Array | null, bias: number, depth: boolean, writeDepth = false) {
    const gl = this.gl
    if (mode === gl.LINES && this.clip) [positions, colors] = clipSegments(positions, colors, this.clip)
    const count = positions.length / 3
    if (!count) return
    gl.useProgram(this.line.program)
    gl.uniformMatrix4fv(this.uniform(this.line, 'uViewProj'), false, viewProj)
    gl.uniform1f(this.uniform(this.line, 'uBias'), bias)
    // Lines get the depth slack that lets them show on surfaces; filled triangles are surfaces.
    gl.uniform3fv(this.uniform(this.line, 'uSlack'), depth && bias === 0 && mode !== gl.TRIANGLES ? this.slack : [0, 0, 0])
    gl.bindVertexArray(this.scratchVao)
    gl.bindBuffer(gl.ARRAY_BUFFER, this.scratch[0])
    gl.bufferData(gl.ARRAY_BUFFER, positions, gl.STREAM_DRAW)
    gl.enableVertexAttribArray(0)
    gl.vertexAttribPointer(0, 3, gl.FLOAT, false, 0, 0)
    gl.bindBuffer(gl.ARRAY_BUFFER, this.scratch[1])
    gl.bufferData(gl.ARRAY_BUFFER, colors, gl.STREAM_DRAW)
    gl.enableVertexAttribArray(1)
    gl.vertexAttribPointer(1, 4, gl.FLOAT, false, 0, 0)
    if (sizes) {
      gl.bindBuffer(gl.ARRAY_BUFFER, this.scratch[2])
      gl.bufferData(gl.ARRAY_BUFFER, sizes, gl.STREAM_DRAW)
      gl.enableVertexAttribArray(2)
      gl.vertexAttribPointer(2, 1, gl.FLOAT, false, 0, 0)
    } else {
      gl.disableVertexAttribArray(2)
      gl.vertexAttrib1f(2, 1)
    }
    if (depth) gl.enable(gl.DEPTH_TEST)
    else gl.disable(gl.DEPTH_TEST)
    gl.depthMask(writeDepth)
    gl.enable(gl.BLEND)
    gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA)
    gl.drawArrays(mode, 0, count)
    gl.depthMask(false)
    gl.disable(gl.BLEND)
    gl.bindVertexArray(null)
  }

  /** The grid floor in `grid`, the scheme's colour (the engine's frame says it). */
  private gridLines(view: OrbitView, grid: number[]) {
    const scale = 10 ** Math.floor(Math.log10(Math.max(view.distance / 4, 0.01)))
    const lines = 40
    const extent = scale * lines
    const cx = Math.round(view.target[0] / scale) * scale
    const cy = Math.round(view.target[1] / scale) * scale
    const positions: number[] = []
    const colors: number[] = []
    const push = (a: number[], b: number[], color: number[], alpha: number) => {
      positions.push(...a, ...b)
      colors.push(...color, alpha, ...color, alpha)
    }
    for (let step = -lines; step <= lines; step += 1) {
      const offset = step * scale
      const [x, y] = [cx + offset, cy + offset]
      const alpha = 0.55 * (1 - (Math.abs(offset) / extent) ** 2)
      if (Math.abs(x) > scale * 0.01) push([x, cy - extent, 0], [x, cy + extent, 0], grid, alpha)
      if (Math.abs(y) > scale * 0.01) push([cx - extent, y, 0], [cx + extent, y, 0], grid, alpha)
    }
    push([cx - extent, 0, 0], [cx + extent, 0, 0], AXIS_X, 0.9)
    push([0, cy - extent, 0], [0, cy + extent, 0], AXIS_Y, 0.9)
    return { positions: new Float32Array(positions), colors: new Float32Array(colors) }
  }

  private faceTexture(object: number, faces: readonly number[], faceCount: number) {
    const gl = this.gl
    const signature = `${faceCount}:${faces.join(',')}`
    if (this.faceSelection && this.faceSelection.object === object && this.faceSelection.faces === signature) return this.faceSelection.texture
    if (this.faceSelection) gl.deleteTexture(this.faceSelection.texture)
    const width = Math.max(1, Math.min(4096, faceCount))
    const height = Math.max(1, Math.ceil(faceCount / width))
    const data = new Uint8Array(width * height)
    for (const face of faces) if (face < data.length) data[face] = 255
    const texture = gl.createTexture()!
    gl.bindTexture(gl.TEXTURE_2D, texture)
    gl.pixelStorei(gl.UNPACK_ALIGNMENT, 1)
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.R8, width, height, 0, gl.RED, gl.UNSIGNED_BYTE, data)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST)
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST)
    this.faceSelection = { texture, object, faces: signature }
    return texture
  }

  private drawMesh(frame: GlFrame, object: GlObject, mesh: DrawnMesh, view: Mat4, viewProj: Mat4, eye: [number, number, number], perspective: boolean, edit: boolean, shadowed: boolean) {
    const gl = this.gl
    const p = this.mesh
    const world = new Float32Array(object.world)
    gl.useProgram(p.program)
    gl.uniformMatrix4fv(this.uniform(p, 'uModel'), false, world)
    gl.uniformMatrix4fv(this.uniform(p, 'uViewProj'), false, viewProj)
    gl.uniformMatrix3fv(this.uniform(p, 'uNormal'), false, normalMatrix(world))
    gl.uniformMatrix4fv(this.uniform(p, 'uView'), false, view)
    gl.uniform1i(this.uniform(p, 'uPerspective'), perspective ? 1 : 0)
    gl.uniform3fv(this.uniform(p, 'uEye'), eye)
    const material = frame.shading === 'material' ? object.material : null
    gl.uniform1i(this.uniform(p, 'uShading'), frame.shading === 'material' ? 1 : 0)
    gl.uniform1i(this.uniform(p, 'uPaint'), object.paint ?? 0)
    this.bindEnvironment(p, frame)
    gl.activeTexture(gl.TEXTURE4)
    gl.bindTexture(gl.TEXTURE_2D, shadowed && this.shadows ? this.shadows.data : this.blankFloat)
    gl.uniform1i(this.uniform(p, 'uShadowData'), 4)
    gl.activeTexture(gl.TEXTURE5)
    gl.bindTexture(gl.TEXTURE_2D, shadowed && this.shadows ? this.shadows.atlas : this.blankFloat)
    gl.uniform1i(this.uniform(p, 'uShadowAtlas'), 5)
    gl.uniform1i(this.uniform(p, 'uShadows'), shadowed ? 1 : 0)
    gl.uniform4fv(this.uniform(p, 'uBase'), material?.baseColor ?? [0.8, 0.8, 0.8, 1])
    gl.uniform1f(this.uniform(p, 'uMetallic'), material?.metallic ?? 0)
    gl.uniform1f(this.uniform(p, 'uRoughness'), material?.roughness ?? 0.5)
    gl.uniform3fv(this.uniform(p, 'uEmission'), material?.emission ?? [0, 0, 0])
    gl.uniform1i(this.uniform(p, 'uAlphaMode'), material?.alphaMode === 'clip' ? 1 : material?.alphaMode === 'blend' ? 2 : 0)
    gl.uniform1f(this.uniform(p, 'uCutoff'), material?.alphaCutoff ?? 0.5)
    gl.uniform1f(this.uniform(p, 'uTransmission'), material?.transmission ?? 0)
    gl.uniform1f(this.uniform(p, 'uIor'), material?.ior ?? 1.5)
    const bind = (slot: number, name: string, flag: string, texture: WebGLTexture | null) => {
      gl.activeTexture(gl.TEXTURE0 + slot)
      gl.bindTexture(gl.TEXTURE_2D, texture)
      gl.uniform1i(this.uniform(p, name), slot)
      gl.uniform1i(this.uniform(p, flag), texture ? 1 : 0)
    }
    bind(0, 'uBaseTex', 'uHasBaseTex', material?.baseColorTexture ? this.texture(material.baseColorTexture, material.baseColorTexture) : null)
    bind(1, 'uMrTex', 'uHasMrTex', material?.metallicRoughnessTexture ? this.texture(`data:${material.metallicRoughnessTexture}`, material.metallicRoughnessTexture) : null)
    bind(2, 'uNormalTex', 'uHasNormalTex', material?.normalTexture ? this.texture(`data:${material.normalTexture}`, material.normalTexture) : null)
    gl.uniform1f(this.uniform(p, 'uNormalStrength'), material?.normalStrength ?? 1)
    const lights = frame.lights.slice(0, 8)
    gl.uniform1i(this.uniform(p, 'uLightCount'), frame.shading === 'material' ? lights.length : 0)
    lights.forEach((light, index) => {
      gl.uniform1i(this.uniform(p, `uLightKind[${index}]`), light.kind === 'sun' ? 1 : light.kind === 'spot' ? 2 : light.kind === 'area' ? 3 : 0)
      gl.uniform3fv(this.uniform(p, `uLightPos[${index}]`), light.position)
      gl.uniform3fv(this.uniform(p, `uLightDir[${index}]`), light.direction)
      gl.uniform3fv(this.uniform(p, `uLightPower[${index}]`), light.power)
      gl.uniform1f(this.uniform(p, `uLightSpot[${index}]`), light.spotSize)
    })
    gl.uniform3fv(this.uniform(p, 'uAmbient'), frame.world.color.map((channel) => channel * frame.world.strength))
    const klass = object.class > 0 ? object.class : object.peer != null ? 3 + object.peer : 0
    gl.uniform1f(this.uniform(p, 'uClass'), klass / 255)
    const editing = edit && frame.edit?.object === object.id
    gl.activeTexture(gl.TEXTURE3)
    gl.bindTexture(gl.TEXTURE_2D, editing && frame.edit ? this.faceTexture(object.id, frame.edit.faces, Math.max(1, mesh.corners)) : null)
    gl.uniform1i(this.uniform(p, 'uFaceSel'), 3)
    gl.uniform1i(this.uniform(p, 'uEditFaces'), editing ? 1 : 0)
    gl.uniform3fv(this.uniform(p, 'uSelColor'), frame.colors.selected)
    // Closed meshes hide their back faces, as the software viewport does (a mirrored world
    // matrix turns the windings over).
    if (mesh.closed) {
      gl.enable(gl.CULL_FACE)
      gl.frontFace(determinant(world) < 0 ? gl.CW : gl.CCW)
    } else gl.disable(gl.CULL_FACE)
    gl.bindVertexArray(mesh.vao)
    gl.drawArrays(gl.TRIANGLES, 0, mesh.corners)
    gl.bindVertexArray(null)
    gl.disable(gl.CULL_FACE)
  }

  /** Draws `frame` as seen from `at`. */
  draw(frame: GlFrame, at: GlView) {
    const gl = this.gl
    if (gl.isContextLost()) return
    const { width, height } = at
    if (this.canvas.width !== width || this.canvas.height !== height) {
      this.canvas.width = width
      this.canvas.height = height
    }
    const targets = this.ensureTargets(width, height)
    const r = at.view
    // Through a camera: the engine's view and projection; otherwise the orbit's.
    const camera = at.camera ? frame.camera : null
    const view = camera ? new Float32Array(camera.viewMatrix) : viewMatrix(r)
    const proj = camera ? new Float32Array(camera.projection) : projection(r, width, height)
    const viewProj = multiply(proj, view)
    const perspective = camera ? camera.perspective : r.perspective
    const eyePoint = camera
      ? camera.eye
      : (() => {
          const [sx, cx, sz, cz] = [Math.sin(r.tilt), Math.cos(r.tilt), Math.sin(r.turn), Math.cos(r.turn)]
          return [r.target[0] + sz * sx * r.distance, r.target[1] - cz * sx * r.distance, r.target[2] + cx * r.distance] as [number, number, number]
        })()
    if (camera && camera.perspective) {
      const forward: [number, number, number] = [-view[2], -view[6], -view[10]]
      this.clip = { eye: camera.eye, forward, near: camera.near * 1.01 }
    } else if (!camera && r.perspective) {
      const length = Math.hypot(r.target[0] - eyePoint[0], r.target[1] - eyePoint[1], r.target[2] - eyePoint[2]) || 1
      const forward: [number, number, number] = [(r.target[0] - eyePoint[0]) / length, (r.target[1] - eyePoint[1]) / length, (r.target[2] - eyePoint[2]) / length]
      this.clip = { eye: eyePoint, forward, near: Math.max(r.distance * 0.001, 0.001) * 1.01 }
    } else {
      this.clip = null
    }
    this.slack = [proj[10], proj[14], perspective ? 1 : 0]
    const drawn = frame.objects.map((object) => ({ object, mesh: this.meshes.get(object.id) })).filter((entry): entry is { object: GlObject; mesh: DrawnMesh } => entry.mesh != null && entry.mesh.key === entry.object.key)
    const shadowed = this.drawShadows(frame, drawn)
    gl.viewport(0, 0, width, height)

    // 1. The scene into the colour and class targets.
    gl.bindFramebuffer(gl.FRAMEBUFFER, targets.scene)
    gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1])
    gl.clearBufferfv(gl.COLOR, 0, [0, 0, 0, 1])
    gl.clearBufferfv(gl.COLOR, 1, [0, 0, 0, 0])
    gl.depthMask(true)
    gl.clearBufferfv(gl.DEPTH, 0, [1])
    gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.NONE])
    gl.disable(gl.DEPTH_TEST)
    gl.depthMask(false)
    gl.useProgram(this.background.program)
    // The background is the editor's, in the host's color scheme (the frame's).
    gl.uniform3fv(this.uniform(this.background, 'uTop'), frame.background.top)
    gl.uniform3fv(this.uniform(this.background, 'uBottom'), frame.background.bottom)
    gl.uniform2f(this.uniform(this.background, 'uSize'), width, height)
    gl.uniformMatrix4fv(this.uniform(this.background, 'uInverse'), false, invert(viewProj))
    this.bindEnvironment(this.background, frame)
    gl.bindVertexArray(this.emptyVao)
    gl.drawArrays(gl.TRIANGLES, 0, 3)

    const edit = frame.edit != null
    if (frame.shading !== 'wireframe') {
      const blended = (object: GlObject) => frame.shading === 'material' && object.material != null && (object.material.alphaMode === 'blend' || object.material.transmission > 0)
      gl.enable(gl.DEPTH_TEST)
      gl.depthFunc(gl.LESS)
      gl.depthMask(true)
      gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.COLOR_ATTACHMENT1])
      if (edit) {
        gl.enable(gl.POLYGON_OFFSET_FILL)
        gl.polygonOffset(1, 1)
      }
      for (const { object, mesh } of drawn) if (mesh.corners && !blended(object)) this.drawMesh(frame, object, mesh, view, viewProj, eyePoint, perspective, edit, shadowed)
      gl.disable(gl.POLYGON_OFFSET_FILL)
      // See-through surfaces last, far to near, over what is behind them.
      const far = drawn
        .filter(({ object, mesh }) => mesh.corners && blended(object))
        .map((entry) => ({ entry, distance: Math.hypot(entry.object.world[12] - eyePoint[0], entry.object.world[13] - eyePoint[1], entry.object.world[14] - eyePoint[2]) }))
        .sort((a, b) => b.distance - a.distance)
      if (far.length) {
        gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.NONE])
        gl.depthMask(false)
        gl.enable(gl.BLEND)
        gl.blendFuncSeparate(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA, gl.ONE, gl.ONE_MINUS_SRC_ALPHA)
        for (const { entry } of far) this.drawMesh(frame, entry.object, entry.mesh, view, viewProj, eyePoint, perspective, edit, shadowed)
        gl.disable(gl.BLEND)
      }
    }
    gl.drawBuffers([gl.COLOR_ATTACHMENT0, gl.NONE])
    if (frame.grid) {
      const grid = this.gridLines(r, frame.colors.grid)
      this.drawLines(gl.LINES, viewProj, grid.positions, grid.colors, null, 0, true)
    }
    // Wires: every edge in wireframe view (seen through), and wire-only meshes (a curve without
    // a bevel) in the shaded views.
    for (const { object, mesh } of drawn) {
      const wireframe = frame.shading === 'wireframe'
      if (!wireframe && mesh.corners) continue
      if (frame.edit?.object === object.id) continue
      const color = object.class === 2 ? frame.colors.active : object.class === 1 ? frame.colors.selected : wireframe ? frame.colors.wire : [0, 0, 0]
      this.drawEdges(mesh, new Float32Array(object.world), viewProj, color, wireframe ? 0.8 : 1, !wireframe)
    }

    // 2. Outlines, into the final picture (which shares the depth).
    gl.bindFramebuffer(gl.FRAMEBUFFER, targets.final)
    gl.drawBuffers([gl.COLOR_ATTACHMENT0])
    gl.disable(gl.DEPTH_TEST)
    gl.depthMask(false)
    gl.useProgram(this.outline.program)
    gl.activeTexture(gl.TEXTURE0)
    gl.bindTexture(gl.TEXTURE_2D, targets.color)
    gl.uniform1i(this.uniform(this.outline, 'uColor'), 0)
    gl.activeTexture(gl.TEXTURE1)
    gl.bindTexture(gl.TEXTURE_2D, targets.klass)
    gl.uniform1i(this.uniform(this.outline, 'uClass'), 1)
    gl.uniform1i(this.uniform(this.outline, 'uSelection'), frame.shading !== 'wireframe' ? 1 : 0)
    gl.uniform3fv(this.uniform(this.outline, 'uSelected'), frame.colors.selected)
    gl.uniform3fv(this.uniform(this.outline, 'uActive'), frame.colors.active)
    const peers = new Float32Array(24)
    frame.peers.slice(0, 8).forEach((peer, index) => peers.set(peer.color, index * 3))
    gl.uniform3fv(this.uniform(this.outline, 'uPeers'), peers)
    gl.uniform1i(this.uniform(this.outline, 'uDim'), camera ? 1 : 0)
    gl.uniform4fv(this.uniform(this.outline, 'uFrame'), camera ? camera.frame : [0, 0, 0, 0])
    gl.uniform3fv(this.uniform(this.outline, 'uOutside'), frame.colors.outside)
    gl.uniform1f(this.uniform(this.outline, 'uHeight'), height)
    gl.bindVertexArray(this.emptyVao)
    gl.drawArrays(gl.TRIANGLES, 0, 3)

    // 3. Overlays: gizmos, lines over everything, edit mode, dots, selected origins.
    const opaque = (colors: number[]) => {
      const out = new Float32Array((colors.length / 3) * 4)
      for (let index = 0; index < colors.length / 3; index += 1) out.set([colors[index * 3], colors[index * 3 + 1], colors[index * 3 + 2], 1], index * 4)
      return out
    }
    // Custom bone shapes' faces hide what is behind them, as the software viewport's do.
    if (frame.fills.positions.length) this.drawLines(gl.TRIANGLES, viewProj, new Float32Array(frame.fills.positions), opaque(frame.fills.colors), null, 0, true, true)
    if (frame.gizmos.positions.length) this.drawLines(gl.LINES, viewProj, new Float32Array(frame.gizmos.positions), opaque(frame.gizmos.colors), null, 0, true)
    if (frame.front.positions.length) this.drawLines(gl.LINES, viewProj, new Float32Array(frame.front.positions), opaque(frame.front.colors), null, 0, false)
    if (frame.edit) {
      const target = drawn.find(({ object }) => object.id === frame.edit?.object)
      if (target) this.drawEditOverlay(frame, target.mesh, new Float32Array(target.object.world), viewProj)
    }
    if (frame.dots.positions.length) this.drawLines(gl.POINTS, viewProj, new Float32Array(frame.dots.positions), opaque(frame.dots.colors), new Float32Array(frame.dots.sizes), 0, false)
    if (frame.origins.length) {
      const positions = new Float32Array(frame.origins.flatMap((origin) => origin.at))
      const colors = new Float32Array(frame.origins.flatMap((origin) => [...(origin.active ? frame.colors.active : frame.colors.selected), 1]))
      this.drawLines(gl.POINTS, viewProj, positions, colors, new Float32Array(frame.origins.map(() => 4)), 0, false)
    }

    // 4. To the screen.
    gl.bindFramebuffer(gl.READ_FRAMEBUFFER, targets.final)
    gl.bindFramebuffer(gl.DRAW_FRAMEBUFFER, null)
    gl.blitFramebuffer(0, 0, width, height, 0, 0, width, height, gl.COLOR_BUFFER_BIT, gl.NEAREST)
    gl.bindFramebuffer(gl.FRAMEBUFFER, null)
  }

  private worldPositions(mesh: DrawnMesh, world: Float32Array) {
    const out = new Float32Array(mesh.positions.length)
    for (let index = 0; index < mesh.positions.length; index += 3) {
      const [x, y, z] = [mesh.positions[index], mesh.positions[index + 1], mesh.positions[index + 2]]
      out[index] = world[0] * x + world[4] * y + world[8] * z + world[12]
      out[index + 1] = world[1] * x + world[5] * y + world[9] * z + world[13]
      out[index + 2] = world[2] * x + world[6] * y + world[10] * z + world[14]
    }
    return out
  }

  private drawEdges(mesh: DrawnMesh, world: Float32Array, viewProj: Mat4, color: readonly number[], alpha: number, depth: boolean) {
    const points = this.worldPositions(mesh, world)
    const positions = new Float32Array(mesh.edges.length * 3)
    const colors = new Float32Array(mesh.edges.length * 4)
    mesh.edges.forEach((vertex, slot) => {
      positions.set(points.subarray(vertex * 3, vertex * 3 + 3), slot * 3)
      colors.set([color[0], color[1], color[2], alpha], slot * 4)
    })
    this.drawLines(this.gl.LINES, viewProj, positions, colors, null, 0, depth)
  }

  private drawEditOverlay(frame: GlFrame, mesh: DrawnMesh, world: Float32Array, viewProj: Mat4) {
    const selected = new Set(frame.edit?.vertices ?? [])
    const points = this.worldPositions(mesh, world)
    const [sel, wire] = [frame.colors.selected, frame.colors.wire]
    const positions = new Float32Array(mesh.edges.length * 3)
    const colors = new Float32Array(mesh.edges.length * 4)
    for (let slot = 0; slot < mesh.edges.length; slot += 2) {
      const [a, b] = [mesh.edges[slot], mesh.edges[slot + 1]]
      const color = selected.has(a) && selected.has(b) ? sel : wire
      positions.set(points.subarray(a * 3, a * 3 + 3), slot * 3)
      positions.set(points.subarray(b * 3, b * 3 + 3), slot * 3 + 3)
      colors.set([...color, 1, ...color, 1], slot * 4)
    }
    this.drawLines(this.gl.LINES, viewProj, positions, colors, null, 2e-5, true)
    const count = points.length / 3
    const vertexColors = new Float32Array(count * 4)
    const sizes = new Float32Array(count)
    for (let index = 0; index < count; index += 1) {
      const on = selected.has(index)
      vertexColors.set([...(on ? sel : EDIT_VERTEX), 1], index * 4)
      sizes[index] = on ? 5 : 4
    }
    this.drawLines(this.gl.POINTS, viewProj, points, vertexColors, sizes, 4e-5, true)
  }

  dispose() {
    for (const mesh of this.meshes.values()) this.free(mesh)
    this.meshes.clear()
    this.setEnvironment('', null)
    this.gl.getExtension('WEBGL_lose_context')?.loseContext()
  }
}

/**
 * Segments cut where they cross the near plane, their part behind it dropped: GPUs differ in how
 * they clip a line whose ends lie on both sides of the eye, and some drop it whole.
 */
function clipSegments(positions: Float32Array, colors: Float32Array, clip: { eye: number[]; forward: number[]; near: number }): [Float32Array, Float32Array] {
  const outPositions = new Float32Array(positions.length)
  const outColors = new Float32Array(colors.length)
  let kept = 0
  const depth = (index: number) =>
    (positions[index * 3] - clip.eye[0]) * clip.forward[0] + (positions[index * 3 + 1] - clip.eye[1]) * clip.forward[1] + (positions[index * 3 + 2] - clip.eye[2]) * clip.forward[2] - clip.near
  for (let a = 0; a + 1 < positions.length / 3; a += 2) {
    const b = a + 1
    const [da, db] = [depth(a), depth(b)]
    if (da < 0 && db < 0) continue
    for (const [from, other, d, e] of [[a, b, da, db], [b, a, db, da]] as const) {
      if (d >= 0) {
        outPositions.set(positions.subarray(from * 3, from * 3 + 3), kept * 3)
      } else {
        const t = d / (d - e)
        for (let axis = 0; axis < 3; axis += 1) outPositions[kept * 3 + axis] = positions[from * 3 + axis] + (positions[other * 3 + axis] - positions[from * 3 + axis]) * t
      }
      outColors.set(colors.subarray(from * 4, from * 4 + 4), kept * 4)
      kept += 1
    }
  }
  return [outPositions.subarray(0, kept * 3), outColors.subarray(0, kept * 4)]
}

/** The determinant of a matrix's upper 3×3 (negative when it mirrors). */
function determinant(m: Float32Array) {
  return m[0] * (m[5] * m[10] - m[9] * m[6]) - m[4] * (m[1] * m[10] - m[9] * m[2]) + m[8] * (m[1] * m[6] - m[5] * m[2])
}

/** The inverse of a column-major 4×4 (zero when it has none). */
function invert(m: Mat4): Float32Array {
  const inv = new Float32Array(16)
  inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15] + m[9] * m[7] * m[14] + m[13] * m[6] * m[11] - m[13] * m[7] * m[10]
  inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15] - m[8] * m[7] * m[14] - m[12] * m[6] * m[11] + m[12] * m[7] * m[10]
  inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15] + m[8] * m[7] * m[13] + m[12] * m[5] * m[11] - m[12] * m[7] * m[9]
  inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14] - m[8] * m[6] * m[13] - m[12] * m[5] * m[10] + m[12] * m[6] * m[9]
  inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15] - m[9] * m[3] * m[14] - m[13] * m[2] * m[11] + m[13] * m[3] * m[10]
  inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15] + m[8] * m[3] * m[14] + m[12] * m[2] * m[11] - m[12] * m[3] * m[10]
  inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15] - m[8] * m[3] * m[13] - m[12] * m[1] * m[11] + m[12] * m[3] * m[9]
  inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14] + m[8] * m[2] * m[13] + m[12] * m[1] * m[10] - m[12] * m[2] * m[9]
  inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15] + m[5] * m[3] * m[14] + m[13] * m[2] * m[7] - m[13] * m[3] * m[6]
  inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15] - m[4] * m[3] * m[14] - m[12] * m[2] * m[7] + m[12] * m[3] * m[6]
  inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15] + m[4] * m[3] * m[13] + m[12] * m[1] * m[7] - m[12] * m[3] * m[5]
  inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14] - m[4] * m[2] * m[13] - m[12] * m[1] * m[6] + m[12] * m[2] * m[5]
  inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11] - m[5] * m[3] * m[10] - m[9] * m[2] * m[7] + m[9] * m[3] * m[6]
  inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11] + m[4] * m[3] * m[10] + m[8] * m[2] * m[7] - m[8] * m[3] * m[6]
  inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11] - m[4] * m[3] * m[9] - m[8] * m[1] * m[7] + m[8] * m[3] * m[5]
  inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10] + m[4] * m[2] * m[9] + m[8] * m[1] * m[6] - m[8] * m[2] * m[5]
  const det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12]
  return Math.abs(det) > 1e-30 ? inv.map((value) => value / det) : new Float32Array(16)
}
