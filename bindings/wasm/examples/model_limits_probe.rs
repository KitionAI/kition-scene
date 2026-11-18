//! Remaining JSON metadata limits after C8-land-12-scene. No model calls or file writes.
use kition_kit::{KitDocument, kdm::Schema, session::model::session_batch};
use kition_scene_kit::SceneDocument;
use serde_json::json;

fn main() -> Result<(), String> {
    // `members` addresses literal immediate keys. It cannot constrain numeric leaves in
    // arrays/nested objects, or a numeric branch of a heterogeneous member.
    for (members, value) in [
        (json!({"resolution":{"minimum":1,"maximum":1024}}), json!({"type":"lattice","resolution":[2,2,2]})),
        (json!({"motionBlur.shutter":{"minimum":0,"maximum":100}}), json!({"motionBlur":{"shutter":-1}})),
        (json!({"baseColor.0":{"minimum":0,"maximum":1}}), json!({"baseColor":[2,0,0,1]})),
    ] {
        let schema = json!({"kdm":1,"kit":"kit.probe","schemaVersion":1,"root":"Root",
            "types":{"Root":{"fields":{"data":{"kind":"json","members":members}}}}});
        let schema = Schema::parse(&schema.to_string()).map_err(|error| error.to_string())?;
        let error = schema.check_value("Root", "data", &value).err().map(|error| error.to_string());
        println!("{}", json!({"members":members,"value":value,"accepted":error.is_none(),"error":error}));
    }
    let mut doc = SceneDocument::create("Probe.kscene", 320, 180)?;
    let result = session_batch(
        &mut doc,
        &json!({"ops":[
            {"op":"set","target":"root","field":"render.motionBlur","value":{"enabled":true,"shutter":0}},
            {"op":"set","target":"root","field":"render.motionBlur.shutter","value":-1}
        ]}),
        false,
    );
    println!(
        "{}",
        json!({"nestedBatchAccepted":result.is_ok(),"error":result.err().map(|error| error.to_string()),
        "engineValidation":doc.scene().validate().err().map(|error| error.to_string())})
    );
    let mut doc = SceneDocument::create("Probe.kscene", 320, 180)?;
    let result =
        session_batch(&mut doc, &json!({"ops":[{"op":"create","type":"Material","in":"root.materials","at":0,"fields":{"name":"Created"}}]}), false);
    println!("{}", json!({"missingJsonCreateAccepted":result.is_ok(),"error":result.err().map(|error| error.to_string())}));
    Ok(())
}
