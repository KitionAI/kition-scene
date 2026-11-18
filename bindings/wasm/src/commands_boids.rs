//! Validated particle boid settings; targets use the same name-or-id convention as objects.
use crate::params::{self, bad, schema as s};
use kition_scene_model::{Scene, particles::ParticleSystem};
use serde_json::{Value, json};

pub fn schema() -> Value {
    json!({"type":"object", "additionalProperties":false, "description":"Airborne boids. Enabled rules are averaged, not evaluated by priority. Up to 2048 particles, 1000 emission-plus-lifetime frames and 500000 particle-frames and 500000 birth/death-aware particle updates. Faces or vertices only; scene force fields and collisions are inert. Targets use current world-space positions.", "properties": {
        "airSpeedMax":s::number("Maximum air speed, 0 to 100 m/s (10)"),
        "airAccMax":s::number("Maximum acceleration as a fraction of maximum speed, 0 to 1 (0.5)"),
        "airAngularMax":s::number("Maximum steering angle as a fraction of 180 degrees, 0 to 1 (0.5)"),
        "airPersonalSpace":s::number("Personal space as a multiple of particle size, 0 to 10 (1)"),
        "rules":{"type":"array", "description":"Up to eight applicable rules averaged; default Separate and Flock. Replaces the complete list.", "items":{"type":"object", "additionalProperties":false, "properties":{
            "kind":s::required(s::choice("Airborne rule", &["goal","avoid","separate","flock"])),
            "target":{"oneOf":[s::integer("Target object's id"),s::string("Target object's name"),{"type":"null"}],"description":"Goal or Avoid target object, null for none"}
        }}}
    }})
}

pub fn configure(scene: &Scene, system: &mut ParticleSystem, params: &Value) -> Result<(), String> {
    if params.get("physics").is_some_and(Value::is_null) {
        return Err(bad("physics must be newtonian or boids"));
    }
    let Some(given) = params.get("boids") else {
        return Ok(());
    };
    let given = given.as_object().ok_or_else(|| bad("boids must be an object of settings"))?;
    let mut value = serde_json::to_value(&system.boids).map_err(|error| error.to_string())?;
    let fields = value.as_object_mut().ok_or_else(|| bad("invalid boid settings"))?;
    for (key, item) in given {
        let mut item = item.clone();
        if key == "rules"
            && let Some(rules) = item.as_array_mut()
        {
            for rule in rules {
                if let Some(target) = rule.get("target").filter(|target| !target.is_null()).cloned()
                    && let Some(fields) = rule.as_object_mut()
                {
                    fields.insert("target".into(), json!(params::resolve(scene, &target)?));
                }
            }
        }
        fields.insert(key.clone(), item);
    }
    system.boids = serde_json::from_value(value).map_err(|error| bad(format!("invalid boid settings: {error}")))?;
    Ok(())
}
