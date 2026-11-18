//! The Scene kit's physics: a rigid body world solved with sequential impulses over convex
//! shapes, cloth and hair solved as position-based dynamics, soft bodies as masses on springs,
//! and the bake that records them into the scene's physics cache frame by frame.

pub mod bake;
pub mod cloth;
pub mod collide;
pub mod hair;
pub mod joint;
pub mod rigid;
pub mod shape;
pub mod soft;

pub use bake::{Bake, bake};
