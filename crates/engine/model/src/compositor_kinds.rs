//! The compositing nodes: what a compositing group (`Tree::Compositor`) is built from. Sockets
//! and settings carry the reference application's names (its compositor's nodes of the same
//! titles, as version 5 has them: menus are settings here, Composite is the group's output); a
//! picture flows through `Color` sockets, and a number or a colour with nothing linked stands for
//! a picture of that value everywhere.

use crate::node_tree::{NodeKind, Setting, Socket, SocketDefault, SocketType, boolean, float, int, named, out, s, vector, when};

const fn color(id: &'static str, value: [f32; 4]) -> Socket {
    s(id, SocketType::Color, SocketDefault::Color(value))
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const IMAGE: Socket = color("Image", WHITE);
const FAC: Socket = named(float("Fac", 1.0), "Factor");

/// The filters of the Blur node.
pub const BLUR_TYPES: &[&str] = &["flat", "tent", "quadratic", "cubic", "gaussian", "fastGaussian", "catrom", "mitch"];
/// The kinds of glare.
pub const GLARE_TYPES: &[&str] = &["kernel", "bloom", "fogGlow", "streaks", "ghosts", "simpleStar", "sunBeams"];
/// The blend modes of the Mix node.
pub const BLEND_TYPES: &[&str] = &[
    "mix",
    "darken",
    "multiply",
    "burn",
    "lighten",
    "screen",
    "dodge",
    "add",
    "overlay",
    "softLight",
    "linearLight",
    "difference",
    "exclusion",
    "subtract",
    "divide",
    "hue",
    "saturation",
    "color",
    "value",
];
const BOOL: &[&str] = &["false", "true"];
const LGG: &[&str] = &["liftGammaGain"];
const CDL: &[&str] = &["offsetPowerSlope"];
const WHITE_POINT: &[&str] = &["whitePoint"];
const STREAKS: &[&str] = &["streaks"];
const STREAKS_GHOSTS: &[&str] = &["streaks", "ghosts"];
const STAR: &[&str] = &["simpleStar"];
const BEAMS: &[&str] = &["sunBeams"];
const RADIAL: &[&str] = &["radial"];
const KERNEL: &[&str] = &["kernel"];
/// How the transform nodes read a picture between pixels, and what it is beyond its edges.
pub const INTERPOLATIONS: &[&str] = &["nearest", "bilinear", "bicubic", "anisotropic"];
pub const EXTENSIONS: &[&str] = &["clip", "extend", "repeat"];
const ANISOTROPIC: &[&str] = &["anisotropic"];
/// The Defocus node's bokeh shapes.
pub const BOKEH: &[&str] = &["circle", "triangle", "square", "pentagon", "hexagon", "heptagon", "octagon"];
/// How the Keying node's feather falls off.
pub const FALLOFFS: &[&str] = &["smooth", "sphere", "root", "inverseSquare", "sharp", "linear"];
const SAMPLING: [Setting; 3] = [
    Setting { id: "Interpolation", choices: INTERPOLATIONS, default: "bilinear" },
    Setting { id: "Extension X", choices: EXTENSIONS, default: "clip" },
    Setting { id: "Extension Y", choices: EXTENSIONS, default: "clip" },
];
const MATTE_OUTPUTS: &[Socket] = &[out("Image", SocketType::Color), out("Matte", SocketType::Float)];
const KEY_COLOR: Socket = color("Key Color", WHITE);

/// Every compositing node kind, in the add menu's order.
pub static COMPOSITOR_KINDS: &[NodeKind] = &[
    NodeKind {
        id: "renderLayers",
        title: "Render Layers",
        category: "Input",
        inputs: &[],
        outputs: &[
            out("Image", SocketType::Color),
            out("Alpha", SocketType::Float),
            out("Depth", SocketType::Float),
            out("Normal", SocketType::Vector),
            out("Vector", SocketType::Vector),
            out("Diffuse Direct", SocketType::Color),
            out("Diffuse Indirect", SocketType::Color),
            out("Diffuse Color", SocketType::Color),
            out("Glossy Direct", SocketType::Color),
            out("Glossy Indirect", SocketType::Color),
            out("Glossy Color", SocketType::Color),
            out("Emission", SocketType::Color),
            out("Environment", SocketType::Color),
            out("Ambient Occlusion", SocketType::Color),
            out("CryptoObject00", SocketType::Color),
            out("CryptoObject01", SocketType::Color),
            out("CryptoObject02", SocketType::Color),
            out("CryptoMaterial00", SocketType::Color),
            out("CryptoMaterial01", SocketType::Color),
            out("CryptoMaterial02", SocketType::Color),
            out("CryptoAsset00", SocketType::Color),
            out("CryptoAsset01", SocketType::Color),
            out("CryptoAsset02", SocketType::Color),
        ],
        settings: &[],
    },
    NodeKind {
        id: "image",
        title: "Image",
        category: "Input",
        inputs: &[s("Image", SocketType::String, SocketDefault::Str(""))],
        outputs: &[out("Image", SocketType::Color), out("Alpha", SocketType::Float)],
        settings: &[],
    },
    NodeKind {
        id: "rgb",
        title: "RGB",
        category: "Input",
        inputs: &[color("Color", [0.5, 0.5, 0.5, 1.0])],
        outputs: &[out("Color", SocketType::Color)],
        settings: &[],
    },
    NodeKind { id: "compositorOutput", title: "Group Output", category: "Output", inputs: &[IMAGE], outputs: &[], settings: &[] },
    NodeKind { id: "viewer", title: "Viewer", category: "Output", inputs: &[color("Image", [0.0, 0.0, 0.0, 1.0])], outputs: &[], settings: &[] },
    NodeKind {
        id: "alphaOver",
        title: "Alpha Over",
        category: "Color",
        inputs: &[color("Background", WHITE), color("Foreground", WHITE), FAC, boolean("Straight Alpha", false)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[Setting { id: "Type", choices: &["over", "disjointOver", "conjointOver"], default: "over" }],
    },
    NodeKind {
        id: "brightContrast",
        title: "Brightness/Contrast",
        category: "Color",
        inputs: &[IMAGE, named(float("Bright", 0.0), "Brightness"), float("Contrast", 0.0)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "colorBalance",
        title: "Color Balance",
        category: "Color",
        inputs: &[
            IMAGE,
            FAC,
            when(named(float("Base Lift", 0.0), "Lift"), "Type", LGG),
            when(named(color("Color Lift", WHITE), "Lift"), "Type", LGG),
            when(named(float("Base Gamma", 1.0), "Gamma"), "Type", LGG),
            when(named(color("Color Gamma", WHITE), "Gamma"), "Type", LGG),
            when(named(float("Base Gain", 1.0), "Gain"), "Type", LGG),
            when(named(color("Color Gain", WHITE), "Gain"), "Type", LGG),
            when(named(float("Base Offset", 0.0), "Offset"), "Type", CDL),
            when(named(color("Color Offset", [0.0, 0.0, 0.0, 1.0]), "Offset"), "Type", CDL),
            when(named(float("Base Power", 1.0), "Power"), "Type", CDL),
            when(named(color("Color Power", WHITE), "Power"), "Type", CDL),
            when(named(float("Base Slope", 1.0), "Slope"), "Type", CDL),
            when(named(color("Color Slope", WHITE), "Slope"), "Type", CDL),
            when(named(float("Input Temperature", 6500.0), "Temperature"), "Type", WHITE_POINT),
            when(named(float("Input Tint", 10.0), "Tint"), "Type", WHITE_POINT),
            when(named(float("Output Temperature", 6500.0), "Temperature"), "Type", WHITE_POINT),
            when(named(float("Output Tint", 10.0), "Tint"), "Type", WHITE_POINT),
        ],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[Setting { id: "Type", choices: &["liftGammaGain", "offsetPowerSlope", "whitePoint"], default: "liftGammaGain" }],
    },
    NodeKind {
        id: "exposure",
        title: "Exposure",
        category: "Color",
        inputs: &[IMAGE, float("Exposure", 0.0)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "hueSaturation",
        title: "Hue/Saturation/Value",
        category: "Color",
        inputs: &[IMAGE, float("Hue", 0.5), float("Saturation", 1.0), float("Value", 1.0), FAC],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "invert",
        title: "Invert Color",
        category: "Color",
        inputs: &[color("Color", WHITE), FAC, boolean("Invert Color", true), boolean("Invert Alpha", false)],
        outputs: &[out("Color", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "mixColor",
        title: "Mix",
        category: "Color",
        inputs: &[float("Factor", 1.0), color("A", [0.5, 0.5, 0.5, 1.0]), color("B", [0.5, 0.5, 0.5, 1.0])],
        outputs: &[out("Result", SocketType::Color)],
        settings: &[
            Setting { id: "blendType", choices: BLEND_TYPES, default: "mix" },
            Setting { id: "clampFactor", choices: BOOL, default: "true" },
            Setting { id: "clampResult", choices: BOOL, default: "false" },
        ],
    },
    NodeKind {
        id: "rgbCurves",
        title: "RGB Curves",
        category: "Color",
        inputs: &[
            IMAGE,
            FAC,
            color("Black Level", [0.0, 0.0, 0.0, 1.0]),
            color("White Level", WHITE),
            s("curves", SocketType::Curves, SocketDefault::None),
        ],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "blur",
        title: "Blur",
        category: "Filter",
        inputs: &[IMAGE, vector("Size", [0.0, 0.0, 0.0]), boolean("Extend Bounds", false)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[Setting { id: "Type", choices: BLUR_TYPES, default: "gaussian" }],
    },
    NodeKind {
        id: "denoise",
        title: "Denoise",
        category: "Filter",
        inputs: &[IMAGE, color("Albedo", WHITE), vector("Normal", [0.0, 0.0, 0.0])],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "glare",
        title: "Glare",
        category: "Filter",
        inputs: &[
            IMAGE,
            named(float("Highlights Threshold", 1.0), "Threshold"),
            named(float("Highlights Smoothness", 0.1), "Smoothness"),
            named(boolean("Clamp Highlights", false), "Clamp"),
            named(float("Maximum Highlights", 10.0), "Maximum"),
            float("Strength", 1.0),
            float("Saturation", 1.0),
            color("Tint", WHITE),
            float("Size", 0.5),
            when(int("Streaks", 4), "Type", STREAKS),
            when(float("Streaks Angle", 0.0), "Type", STREAKS),
            when(int("Iterations", 3), "Type", STREAKS_GHOSTS),
            when(float("Fade", 0.9), "Type", STREAKS),
            when(float("Color Modulation", 0.25), "Type", STREAKS_GHOSTS),
            when(named(boolean("Diagonal Star", true), "Diagonal"), "Type", STAR),
            when(vector("Sun Position", [0.5, 0.5, 0.0]), "Type", BEAMS),
            when(float("Jitter", 0.0), "Type", BEAMS),
            when(named(float("Float Kernel", 0.0), "Kernel"), "Type", KERNEL),
            when(named(color("Color Kernel", [0.8, 0.8, 0.8, 1.0]), "Kernel"), "Type", KERNEL),
        ],
        outputs: &[out("Image", SocketType::Color), out("Glare", SocketType::Color), out("Highlights", SocketType::Color)],
        settings: &[
            Setting { id: "Type", choices: GLARE_TYPES, default: "streaks" },
            Setting { id: "Quality", choices: &["low", "medium", "high"], default: "medium" },
            Setting { id: "Kernel Data Type", choices: &["float", "color"], default: "float" },
        ],
    },
    NodeKind {
        id: "defocus",
        title: "Defocus",
        category: "Filter",
        inputs: &[
            IMAGE,
            float("Z", 1.0),
            float("F-Stop", 128.0),
            float("Max Blur", 16.0),
            boolean("Use Z-Buffer", false),
            float("Z Scale", 1.0),
            float("Angle", 0.0),
        ],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[Setting { id: "Bokeh", choices: BOKEH, default: "circle" }],
    },
    NodeKind {
        id: "vectorBlur",
        title: "Vector Blur",
        category: "Filter",
        inputs: &[IMAGE, vector("Speed", [0.0, 0.0, 0.0]), named(float("Z", 0.0), "Depth"), int("Samples", 32), float("Shutter", 0.5)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "kuwahara",
        title: "Kuwahara",
        category: "Filter",
        inputs: &[
            IMAGE,
            float("Size", 6.0),
            when(int("Uniformity", 4), "Type", ANISOTROPIC),
            when(float("Sharpness", 1.0), "Type", ANISOTROPIC),
            when(float("Eccentricity", 1.0), "Type", ANISOTROPIC),
            boolean("High Precision", false),
        ],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[Setting { id: "Type", choices: &["classic", "anisotropic"], default: "anisotropic" }],
    },
    NodeKind {
        id: "cryptomatte",
        title: "Cryptomatte",
        category: "Keying",
        inputs: &[color("Image", [0.0, 0.0, 0.0, 1.0]), named(s("Matte ID", SocketType::String, SocketDefault::Str("")), "Matte")],
        outputs: &[out("Image", SocketType::Color), out("Matte", SocketType::Float), out("Pick", SocketType::Color)],
        settings: &[Setting { id: "Layer", choices: &["object", "material", "asset"], default: "object" }],
    },
    NodeKind {
        id: "chromaKey",
        title: "Chroma Key",
        category: "Keying",
        inputs: &[IMAGE, KEY_COLOR, float("Minimum", 0.174_532_92), float("Maximum", std::f32::consts::FRAC_PI_6), float("Falloff", 1.0)],
        outputs: MATTE_OUTPUTS,
        settings: &[],
    },
    NodeKind {
        id: "colorKey",
        title: "Color Key",
        category: "Keying",
        inputs: &[IMAGE, KEY_COLOR, float("Hue", 0.01), float("Saturation", 0.1), float("Value", 0.1)],
        outputs: MATTE_OUTPUTS,
        settings: &[],
    },
    NodeKind {
        id: "differenceKey",
        title: "Difference Key",
        category: "Keying",
        inputs: &[color("Image 1", WHITE), color("Image 2", WHITE), float("Tolerance", 0.1), float("Falloff", 0.1)],
        outputs: MATTE_OUTPUTS,
        settings: &[],
    },
    NodeKind {
        id: "distanceKey",
        title: "Distance Key",
        category: "Keying",
        inputs: &[IMAGE, KEY_COLOR, float("Tolerance", 0.1), float("Falloff", 0.1)],
        outputs: MATTE_OUTPUTS,
        settings: &[Setting { id: "Color Space", choices: &["rgb", "ycc"], default: "rgb" }],
    },
    NodeKind {
        id: "keying",
        title: "Keying",
        category: "Keying",
        inputs: &[
            color("Image", [0.8, 0.8, 0.8, 1.0]),
            KEY_COLOR,
            named(int("Preprocess Blur Size", 0), "Blur Size"),
            named(float("Key Balance", 0.5), "Balance"),
            float("Black Level", 0.0),
            float("White Level", 1.0),
            named(int("Edge Search Size", 3), "Size"),
            named(float("Edge Tolerance", 0.1), "Tolerance"),
            float("Garbage Matte", 0.0),
            float("Core Matte", 0.0),
            named(int("Postprocess Blur Size", 0), "Blur Size"),
            named(int("Postprocess Dilate Size", 0), "Dilate Size"),
            named(int("Postprocess Feather Size", 0), "Feather Size"),
            named(float("Despill Strength", 1.0), "Strength"),
            named(float("Despill Balance", 0.5), "Balance"),
        ],
        outputs: &[out("Image", SocketType::Color), out("Matte", SocketType::Float), out("Edges", SocketType::Float)],
        settings: &[Setting { id: "Feather Falloff", choices: FALLOFFS, default: "smooth" }],
    },
    NodeKind {
        id: "luminanceKey",
        title: "Luminance Key",
        category: "Keying",
        inputs: &[IMAGE, float("Minimum", 0.0), float("Maximum", 1.0)],
        outputs: MATTE_OUTPUTS,
        settings: &[],
    },
    NodeKind {
        id: "ellipseMask",
        title: "Ellipse Mask",
        category: "Mask",
        inputs: &[
            float("Mask", 0.0),
            float("Value", 1.0),
            vector("Position", [0.5, 0.5, 0.0]),
            vector("Size", [0.2, 0.1, 0.0]),
            float("Rotation", 0.0),
        ],
        outputs: &[out("Mask", SocketType::Float)],
        settings: &[Setting { id: "Operation", choices: &["add", "subtract", "multiply", "not"], default: "add" }],
    },
    NodeKind {
        id: "lensDistortion",
        title: "Lens Distortion",
        category: "Transform",
        inputs: &[
            IMAGE,
            when(float("Distortion", 0.0), "Type", RADIAL),
            float("Dispersion", 0.0),
            boolean("Jitter", false),
            when(boolean("Fit", false), "Type", RADIAL),
        ],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[Setting { id: "Type", choices: &["radial", "horizontal"], default: "radial" }],
    },
    NodeKind {
        id: "crop",
        title: "Crop",
        category: "Transform",
        inputs: &[IMAGE, int("X", 0), int("Y", 0), int("Width", 1920), int("Height", 1080), boolean("Alpha Crop", false)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "flip",
        title: "Flip",
        category: "Transform",
        inputs: &[IMAGE, boolean("Flip X", false), boolean("Flip Y", false)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[],
    },
    NodeKind {
        id: "rotate",
        title: "Rotate",
        category: "Transform",
        inputs: &[IMAGE, float("Angle", 0.0)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &SAMPLING,
    },
    NodeKind {
        id: "scale",
        title: "Scale",
        category: "Transform",
        inputs: &[IMAGE, float("X", 1.0), float("Y", 1.0)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &[
            Setting { id: "Type", choices: &["relative", "absolute", "sceneSize", "renderSize"], default: "relative" },
            Setting { id: "Frame Type", choices: &["stretch", "fit", "crop"], default: "stretch" },
            SAMPLING[0],
            SAMPLING[1],
            SAMPLING[2],
        ],
    },
    NodeKind {
        id: "imageTransform",
        title: "Transform",
        category: "Transform",
        inputs: &[color("Image", [0.8, 0.8, 0.8, 1.0]), float("X", 0.0), float("Y", 0.0), float("Angle", 0.0), float("Scale", 1.0)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &SAMPLING,
    },
    NodeKind {
        id: "translate",
        title: "Translate",
        category: "Transform",
        inputs: &[IMAGE, float("X", 0.0), float("Y", 0.0)],
        outputs: &[out("Image", SocketType::Color)],
        settings: &SAMPLING,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node_tree::{NODE_KINDS, Tree, in_tree, kinds_of, node_kind};

    #[test]
    fn compositing_kinds_are_their_own_and_the_shared_value_is_in_both_trees() {
        for kind in COMPOSITOR_KINDS {
            assert!(!NODE_KINDS.iter().any(|other| other.id == kind.id), "{} is a geometry node id too", kind.id);
            assert!(std::ptr::eq(node_kind(kind.id).unwrap(), kind));
            assert!(in_tree(kind.id, Tree::Compositor) && !in_tree(kind.id, Tree::Geometry));
            for setting in kind.settings {
                assert!(setting.choices.contains(&setting.default), "{}: {}", kind.id, setting.id);
            }
        }
        assert!(in_tree("value", Tree::Compositor) && in_tree("value", Tree::Geometry));
        assert!(!in_tree("meshCube", Tree::Compositor));
        assert_eq!(kinds_of(Tree::Compositor).len(), COMPOSITOR_KINDS.len() + 1);
    }
}
