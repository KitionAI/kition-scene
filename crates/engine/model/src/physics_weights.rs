//! Per-system field weights, observed through Blender's public EffectorWeights API.
use serde::{Deserialize, Serialize};

use crate::physics::{Effector, FieldKind};
use crate::{Result, SceneError};

/// Cloth and soft-body influence controls. Gravity is independent of `all`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default, deny_unknown_fields)]
pub struct EffectorWeights {
    pub all: f32,
    pub gravity: f32,
    pub force: f32,
    pub wind: f32,
    pub vortex: f32,
    pub turbulence: f32,
    pub drag: f32,
}

impl Default for EffectorWeights {
    fn default() -> Self {
        Self { all: 1.0, gravity: 1.0, force: 1.0, wind: 1.0, vortex: 1.0, turbulence: 1.0, drag: 1.0 }
    }
}

impl EffectorWeights {
    pub fn check(&self) -> Result<()> {
        for (name, value) in [
            ("all", self.all),
            ("gravity", self.gravity),
            ("force", self.force),
            ("wind", self.wind),
            ("vortex", self.vortex),
            ("turbulence", self.turbulence),
            ("drag", self.drag),
        ] {
            if !value.is_finite() || !(-200.0..=200.0).contains(&value) {
                return Err(SceneError::params(format!("effectorWeights.{name} must be between -200 and 200")));
            }
        }
        Ok(())
    }

    /// A nonpositive product disables the field; two negative weights can enable it.
    pub fn factor(&self, kind: FieldKind) -> f32 {
        (self.all
            * match kind {
                FieldKind::Force => self.force,
                FieldKind::Wind => self.wind,
                FieldKind::Vortex => self.vortex,
                FieldKind::Turbulence => self.turbulence,
                FieldKind::Drag => self.drag,
            })
        .max(0.0)
    }

    /// Weight strength and flow before evaluating the field, including Vortex's nonlinear
    /// turning term. Drag uses its two drag coefficients instead of strength.
    pub fn placed(&self, effectors: &[Effector], owner: u32) -> Result<Vec<Effector>> {
        self.check()?;
        let mut out = Vec::new();
        for effector in effectors {
            let factor = self.factor(effector.field.kind);
            if effector.owner == owner || factor <= 0.0 {
                continue;
            }
            let mut weighted = effector.clone();
            if weighted.field.kind == FieldKind::Drag {
                weighted.field.linear_drag *= factor;
                weighted.field.quadratic_drag *= factor;
            } else {
                weighted.field.strength *= factor;
                weighted.field.flow *= factor;
            }
            if ![weighted.field.strength, weighted.field.flow, weighted.field.linear_drag, weighted.field.quadratic_drag]
                .into_iter()
                .all(f32::is_finite)
            {
                return Err(SceneError::params("weighted force-field coefficients must be finite"));
            }
            out.push(weighted);
        }
        Ok(out)
    }
}
