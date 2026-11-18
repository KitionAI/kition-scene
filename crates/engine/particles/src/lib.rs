//! The Scene kit's particle systems, in the manner of the reference application's: emitters whose
//! particles are born over a frame range and fly under gravity, and hair whose strands grow from
//! the surface and can be groomed.
//!
//! Simulated particles are derived data: a particle at a frame comes from the system's settings
//! and seed alone ([`sim`]), so it is never saved and every replica shows the same particles.
//! Groomed hair is saved: it is the person's work ([`blob`]).
#![forbid(unsafe_code)]

pub mod blob;
pub mod boids;
pub mod effector;
pub mod hair;
pub mod rng;
pub mod settings;
pub mod sim;
pub mod surface;

pub use boids::{Boids, Physics as ParticlePhysics};
pub use effector::{Effector, FieldKind, ForceField};
pub use hair::{GroomBrush, GroomDab, Strand};
pub use settings::{Children, Distribution, EmitFrom, HairDynamics, Kind, ParticleSystem, RenderAs};
pub use sim::{Cache, Field, Particle};
pub use surface::{Start, Surface};
