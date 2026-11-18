//! Partial, strictly typed updates to cloth and soft-body field weights.
use kition_scene_model::physics::EffectorWeights;
use serde_json::{Value, json};

use crate::params::{bad, schema as s};

pub fn schema() -> Value {
    let properties = s::of(vec![
        ("all", s::number("All field weights multiplier, -200 to 200 (1); does not affect gravity")),
        ("gravity", s::number("Signed gravity multiplier, -200 to 200 (1), independent of All")),
        ("force", s::number("Force weight, -200 to 200 (1)")),
        ("wind", s::number("Wind weight, -200 to 200 (1)")),
        ("vortex", s::number("Vortex weight, -200 to 200 (1)")),
        ("turbulence", s::number("Turbulence weight, -200 to 200 (1)")),
        ("drag", s::number("Drag weight, -200 to 200 (1)")),
    ]);
    json!({"type":"object","additionalProperties":false,"description":"Partial cloth or soft-body influence weights; All multiplies field weights, Gravity is independent", "properties":properties})
}

pub fn configure(weights: &mut EffectorWeights, params: &Value) -> Result<(), String> {
    let Some(given) = params.get("effectorWeights") else { return Ok(()) };
    let given = given.as_object().ok_or_else(|| bad("effectorWeights must be an object of numeric weights"))?;
    let mut value = serde_json::to_value(*weights).map_err(|error| bad(error.to_string()))?;
    let fields = value.as_object_mut().ok_or_else(|| bad("invalid effector weights"))?;
    for (key, value) in given {
        if !fields.contains_key(key) {
            return Err(bad(format!("unknown effector weight `{key}`")));
        }
        if !value.is_number() {
            return Err(bad(format!("effectorWeights.{key} must be a number")));
        }
        fields.insert(key.clone(), value.clone());
    }
    let updated: EffectorWeights = serde_json::from_value(value).map_err(|error| bad(error.to_string()))?;
    updated.check().map_err(|error| bad(error.to_string()))?;
    *weights = updated;
    Ok(())
}
