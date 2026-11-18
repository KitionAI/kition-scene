//! The shader nodes: what a material's shader group (`Tree::Shader`) is built from. Sockets
//! carry the reference application's socket identifiers and defaults (its shader nodes of the
//! same titles, as version 5 has them); its menus are settings here, with lower-case choices. A
//! surface flows through `Shader` sockets into the Material Output; an unlinked Normal input is
//! the surface's own normal, and an unlinked texture Vector the generated coordinates (UV for
//! Image Texture), as in the reference.

use crate::node_tree::{Implicit, NodeKind, Setting, Socket, SocketDefault, SocketType, float, implicit, named, out, s, vector, when};

const fn color(id: &'static str, value: [f32; 4]) -> Socket {
    s(id, SocketType::Color, SocketDefault::Color(value))
}

const fn shader(id: &'static str) -> Socket {
    s(id, SocketType::Shader, SocketDefault::None)
}

const GREY: [f32; 4] = [0.8, 0.8, 0.8, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
/// A Normal input: the surface's normal when nothing links into it.
const NORMAL: Socket = implicit(vector("Normal", [0.0, 0.0, 0.0]), Implicit::Normal);
/// A procedural texture's Vector: the generated coordinates when nothing links into it.
const GENERATED: Socket = implicit(vector("Vector", [0.0, 0.0, 0.0]), Implicit::Generated);
const BOOL: &[&str] = &["false", "true"];

/// Microfacet distributions.
pub const DISTRIBUTIONS: &[&str] = &["ggx", "multi_ggx"];
/// The Gradient Texture's kinds.
pub const GRADIENT_TYPES: &[&str] = &["linear", "quadratic", "easing", "diagonal", "spherical", "quadratic_sphere", "radial"];
/// The Noise Texture's fractal kinds.
pub const NOISE_TYPES: &[&str] = &["fbm", "multifractal", "ridged_multifractal", "hybrid_multifractal", "hetero_terrain"];
/// The dimensions procedural textures work in.
pub const DIMENSIONS: &[&str] = &["1d", "2d", "3d"];
/// Voronoi distance metrics.
pub const DISTANCES: &[&str] = &["euclidean", "manhattan", "chebychev", "minkowski"];
/// Voronoi features.
pub const FEATURES: &[&str] = &["f1", "f2", "smooth_f1", "distance_to_edge", "n_sphere_radius"];
const NOT_EDGE: &[&str] = &["f1", "f2", "smooth_f1"];
const MUSGRAVE_OFFSET: &[&str] = &["ridged_multifractal", "hybrid_multifractal", "hetero_terrain"];
const MUSGRAVE_GAIN: &[&str] = &["ridged_multifractal", "hybrid_multifractal"];
/// How Color Ramp blends between its stops.
pub const RAMP_INTERPOLATIONS: &[&str] = &["linear", "constant", "ease"];
/// Volume Scatter's phase functions: how scattered light spreads around its direction.
pub const PHASES: &[&str] = &["henyey_greenstein", "rayleigh"];
/// Colour models of Separate and Combine Color.
pub const COLOR_MODES: &[&str] = &["rgb", "hsv", "hsl"];

/// Every shader node kind, in the add menu's order.
pub static SHADER_KINDS: &[NodeKind] = &[
    NodeKind {
        id: "materialOutput",
        title: "Material Output",
        category: "Output",
        inputs: &[shader("Surface"), shader("Volume")],
        outputs: &[],
        settings: &[],
    },
    NodeKind {
        id: "textureCoordinate",
        title: "Texture Coordinate",
        category: "Input",
        inputs: &[],
        outputs: &[
            out("Generated", SocketType::Vector),
            out("Normal", SocketType::Vector),
            out("UV", SocketType::Vector),
            out("Object", SocketType::Vector),
            out("Reflection", SocketType::Vector),
        ],
        settings: &[],
    },
    NodeKind {
        id: "geometryInfo",
        title: "Geometry",
        category: "Input",
        inputs: &[],
        outputs: &[
            out("Position", SocketType::Vector),
            out("Normal", SocketType::Vector),
            out("Tangent", SocketType::Vector),
            out("True Normal", SocketType::Vector),
            out("Incoming", SocketType::Vector),
            out("Backfacing", SocketType::Float),
        ],
        settings: &[],
    },
    NodeKind {
        id: "fresnel",
        title: "Fresnel",
        category: "Input",
        inputs: &[float("IOR", 1.5), NORMAL],
        outputs: &[named(out("Fac", SocketType::Float), "Factor")],
        settings: &[],
    },
    NodeKind {
        id: "layerWeight",
        title: "Layer Weight",
        category: "Input",
        inputs: &[float("Blend", 0.5), NORMAL],
        outputs: &[out("Fresnel", SocketType::Float), out("Facing", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "principledBsdf",
        title: "Principled BSDF",
        category: "Shader",
        inputs: &[
            color("Base Color", GREY),
            float("Metallic", 0.0),
            float("Roughness", 0.5),
            float("IOR", 1.5),
            float("Alpha", 1.0),
            NORMAL,
            float("Subsurface Weight", 0.0),
            vector("Subsurface Radius", [1.0, 0.2, 0.1]),
            float("Subsurface Scale", 0.005),
            float("Specular IOR Level", 0.5),
            color("Specular Tint", WHITE),
            float("Anisotropic", 0.0),
            float("Anisotropic Rotation", 0.0),
            vector("Tangent", [0.0, 0.0, 0.0]),
            float("Transmission Weight", 0.0),
            float("Coat Weight", 0.0),
            float("Coat Roughness", 0.03),
            float("Coat IOR", 1.5),
            color("Coat Tint", WHITE),
            implicit(vector("Coat Normal", [0.0, 0.0, 0.0]), Implicit::Normal),
            float("Sheen Weight", 0.0),
            float("Sheen Roughness", 0.5),
            color("Sheen Tint", WHITE),
            color("Emission Color", WHITE),
            float("Emission Strength", 0.0),
        ],
        outputs: &[out("BSDF", SocketType::Shader)],
        settings: &[Setting { id: "distribution", choices: DISTRIBUTIONS, default: "multi_ggx" }],
    },
    NodeKind {
        id: "diffuseBsdf",
        title: "Diffuse BSDF",
        category: "Shader",
        inputs: &[color("Color", GREY), float("Roughness", 0.0), NORMAL],
        outputs: &[out("BSDF", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "glossyBsdf",
        title: "Glossy BSDF",
        category: "Shader",
        inputs: &[
            color("Color", GREY),
            float("Roughness", 0.5),
            float("Anisotropy", 0.0),
            float("Rotation", 0.0),
            NORMAL,
            vector("Tangent", [0.0, 0.0, 0.0]),
        ],
        outputs: &[out("BSDF", SocketType::Shader)],
        settings: &[Setting { id: "distribution", choices: DISTRIBUTIONS, default: "multi_ggx" }],
    },
    NodeKind {
        id: "glassBsdf",
        title: "Glass BSDF",
        category: "Shader",
        inputs: &[color("Color", WHITE), float("Roughness", 0.0), float("IOR", 1.5), NORMAL],
        outputs: &[out("BSDF", SocketType::Shader)],
        settings: &[Setting { id: "distribution", choices: DISTRIBUTIONS, default: "multi_ggx" }],
    },
    NodeKind {
        id: "transparentBsdf",
        title: "Transparent BSDF",
        category: "Shader",
        inputs: &[color("Color", WHITE)],
        outputs: &[out("BSDF", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "emission",
        title: "Emission",
        category: "Shader",
        inputs: &[color("Color", WHITE), float("Strength", 1.0)],
        outputs: &[out("Emission", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "volumeAbsorption",
        title: "Volume Absorption",
        category: "Shader",
        inputs: &[color("Color", GREY), float("Density", 1.0)],
        outputs: &[out("Volume", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "volumeScatter",
        title: "Volume Scatter",
        category: "Shader",
        inputs: &[color("Color", GREY), float("Density", 1.0), float("Anisotropy", 0.0)],
        outputs: &[out("Volume", SocketType::Shader)],
        settings: &[Setting { id: "phase", choices: PHASES, default: "henyey_greenstein" }],
    },
    NodeKind {
        id: "principledVolume",
        title: "Principled Volume",
        category: "Shader",
        inputs: &[
            color("Color", [0.5, 0.5, 0.5, 1.0]),
            float("Density", 1.0),
            float("Anisotropy", 0.0),
            color("Absorption Color", [0.0, 0.0, 0.0, 1.0]),
            float("Emission Strength", 0.0),
            color("Emission Color", WHITE),
        ],
        outputs: &[out("Volume", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "mixShader",
        title: "Mix Shader",
        category: "Shader",
        inputs: &[named(float("Fac", 0.5), "Factor"), shader("Shader"), named(shader("Shader_001"), "Shader")],
        outputs: &[out("Shader", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "addShader",
        title: "Add Shader",
        category: "Shader",
        inputs: &[shader("Shader"), named(shader("Shader_001"), "Shader")],
        outputs: &[out("Shader", SocketType::Shader)],
        settings: &[],
    },
    NodeKind {
        id: "imageTexture",
        title: "Image Texture",
        category: "Texture",
        inputs: &[implicit(vector("Vector", [0.0, 0.0, 0.0]), Implicit::Uv), s("Image", SocketType::String, SocketDefault::Str(""))],
        outputs: &[out("Color", SocketType::Color), out("Alpha", SocketType::Float)],
        settings: &[
            Setting { id: "interpolation", choices: &["linear", "closest"], default: "linear" },
            Setting { id: "extension", choices: &["repeat", "extend", "clip", "mirror"], default: "repeat" },
            Setting { id: "colorSpace", choices: &["srgb", "non_color"], default: "srgb" },
        ],
    },
    NodeKind {
        id: "noiseTexture",
        title: "Noise Texture",
        category: "Texture",
        inputs: &[
            GENERATED,
            float("Scale", 5.0),
            float("Detail", 2.0),
            float("Roughness", 0.5),
            float("Lacunarity", 2.0),
            when(float("Offset", 0.0), "noiseType", MUSGRAVE_OFFSET),
            when(float("Gain", 1.0), "noiseType", MUSGRAVE_GAIN),
            float("Distortion", 0.0),
        ],
        outputs: &[named(out("Fac", SocketType::Float), "Factor"), out("Color", SocketType::Color)],
        settings: &[
            Setting { id: "noiseDimensions", choices: DIMENSIONS, default: "3d" },
            Setting { id: "noiseType", choices: NOISE_TYPES, default: "fbm" },
            Setting { id: "normalize", choices: BOOL, default: "true" },
        ],
    },
    NodeKind {
        id: "voronoiTexture",
        title: "Voronoi Texture",
        category: "Texture",
        inputs: &[
            GENERATED,
            float("Scale", 5.0),
            when(float("Detail", 0.0), "feature", NOT_EDGE),
            when(float("Roughness", 0.5), "feature", NOT_EDGE),
            when(float("Lacunarity", 2.0), "feature", NOT_EDGE),
            when(float("Smoothness", 1.0), "feature", &["smooth_f1"]),
            when(float("Exponent", 0.5), "distance", &["minkowski"]),
            float("Randomness", 1.0),
        ],
        outputs: &[
            when(out("Distance", SocketType::Float), "feature", &["f1", "f2", "smooth_f1", "distance_to_edge"]),
            when(out("Color", SocketType::Color), "feature", NOT_EDGE),
            when(out("Position", SocketType::Vector), "feature", NOT_EDGE),
            when(out("Radius", SocketType::Float), "feature", &["n_sphere_radius"]),
        ],
        settings: &[
            Setting { id: "voronoiDimensions", choices: DIMENSIONS, default: "3d" },
            Setting { id: "distance", choices: DISTANCES, default: "euclidean" },
            Setting { id: "feature", choices: FEATURES, default: "f1" },
            Setting { id: "normalize", choices: BOOL, default: "false" },
        ],
    },
    NodeKind {
        id: "gradientTexture",
        title: "Gradient Texture",
        category: "Texture",
        inputs: &[GENERATED],
        outputs: &[out("Color", SocketType::Color), named(out("Fac", SocketType::Float), "Factor")],
        settings: &[Setting { id: "gradientType", choices: GRADIENT_TYPES, default: "linear" }],
    },
    NodeKind {
        id: "checkerTexture",
        title: "Checker Texture",
        category: "Texture",
        inputs: &[GENERATED, color("Color1", GREY), color("Color2", [0.2, 0.2, 0.2, 1.0]), float("Scale", 5.0)],
        outputs: &[out("Color", SocketType::Color), named(out("Fac", SocketType::Float), "Factor")],
        settings: &[],
    },
    NodeKind {
        id: "waveTexture",
        title: "Wave Texture",
        category: "Texture",
        inputs: &[
            GENERATED,
            float("Scale", 5.0),
            float("Distortion", 0.0),
            float("Detail", 2.0),
            float("Detail Scale", 1.0),
            float("Detail Roughness", 0.5),
            float("Phase Offset", 0.0),
        ],
        outputs: &[out("Color", SocketType::Color), named(out("Fac", SocketType::Float), "Factor")],
        settings: &[
            Setting { id: "waveType", choices: &["bands", "rings"], default: "bands" },
            Setting { id: "bandsDirection", choices: &["x", "y", "z", "diagonal"], default: "x" },
            Setting { id: "ringsDirection", choices: &["x", "y", "z", "spherical"], default: "x" },
            Setting { id: "waveProfile", choices: &["sin", "saw", "tri"], default: "sin" },
        ],
    },
    NodeKind {
        id: "colorRamp",
        title: "Color Ramp",
        category: "Converter",
        inputs: &[named(float("Fac", 0.5), "Factor"), s("ramp", SocketType::Ramp, SocketDefault::None)],
        outputs: &[out("Color", SocketType::Color), out("Alpha", SocketType::Float)],
        settings: &[Setting { id: "interpolation", choices: RAMP_INTERPOLATIONS, default: "linear" }],
    },
    NodeKind {
        id: "separateColor",
        title: "Separate Color",
        category: "Converter",
        inputs: &[color("Color", GREY)],
        outputs: &[out("Red", SocketType::Float), out("Green", SocketType::Float), out("Blue", SocketType::Float)],
        settings: &[Setting { id: "mode", choices: COLOR_MODES, default: "rgb" }],
    },
    NodeKind {
        id: "combineColor",
        title: "Combine Color",
        category: "Converter",
        inputs: &[float("Red", 0.0), float("Green", 0.0), float("Blue", 0.0)],
        outputs: &[out("Color", SocketType::Color)],
        settings: &[Setting { id: "mode", choices: COLOR_MODES, default: "rgb" }],
    },
    NodeKind {
        id: "mapping",
        title: "Mapping",
        category: "Vector",
        inputs: &[
            vector("Vector", [0.0, 0.0, 0.0]),
            when(vector("Location", [0.0, 0.0, 0.0]), "vectorType", &["point", "texture"]),
            vector("Rotation", [0.0, 0.0, 0.0]),
            vector("Scale", [1.0, 1.0, 1.0]),
        ],
        outputs: &[out("Vector", SocketType::Vector)],
        settings: &[Setting { id: "vectorType", choices: &["point", "texture", "vector", "normal"], default: "point" }],
    },
    NodeKind {
        id: "bump",
        title: "Bump",
        category: "Vector",
        inputs: &[float("Strength", 1.0), float("Distance", 0.001), float("Height", 1.0), NORMAL],
        outputs: &[out("Normal", SocketType::Vector)],
        settings: &[Setting { id: "invert", choices: BOOL, default: "false" }],
    },
    NodeKind {
        id: "normalMap",
        title: "Normal Map",
        category: "Vector",
        inputs: &[float("Strength", 1.0), color("Color", [0.5, 0.5, 1.0, 1.0])],
        outputs: &[out("Normal", SocketType::Vector)],
        settings: &[Setting { id: "space", choices: &["tangent", "object", "world"], default: "tangent" }],
    },
];

/// Kinds of the other trees a shader group also takes: inputs, colour, and maths nodes that
/// work the same on a surface point.
pub const SHARED_WITH_SHADERS: &[&str] = &["value", "rgb", "mixColor", "invert", "math", "vectorMath", "separateXyz", "combineXyz", "mapRange"];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_tree::{NODE_KINDS, Tree, in_tree, kinds_of, node_kind};

    #[test]
    fn shader_kinds_are_their_own_and_shared_ones_are_known() {
        for kind in SHADER_KINDS {
            assert!(!NODE_KINDS.iter().any(|other| other.id == kind.id), "{} is a geometry node id too", kind.id);
            assert!(std::ptr::eq(node_kind(kind.id).unwrap(), kind));
            assert!(in_tree(kind.id, Tree::Shader) && !in_tree(kind.id, Tree::Geometry) && !in_tree(kind.id, Tree::Compositor));
            for setting in kind.settings {
                assert!(setting.choices.contains(&setting.default), "{}: {}", kind.id, setting.id);
            }
            for socket in kind.inputs.iter().chain(kind.outputs) {
                if let Some((setting, values)) = socket.when {
                    let known = kind.settings.iter().find(|item| item.id == setting).unwrap_or_else(|| panic!("{}: no setting {setting}", kind.id));
                    for value in values {
                        assert!(known.choices.contains(value), "{}: {setting} has no {value}", kind.id);
                    }
                }
            }
        }
        for id in SHARED_WITH_SHADERS {
            assert!(node_kind(id).is_some() && in_tree(id, Tree::Shader), "{id}");
        }
        assert!(!in_tree("meshCube", Tree::Shader) && !in_tree("glare", Tree::Shader));
        assert_eq!(kinds_of(Tree::Shader).len(), SHADER_KINDS.len() + SHARED_WITH_SHADERS.len());
    }
}
