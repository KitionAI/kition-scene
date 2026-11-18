//! Damaged and hostile FBX files are refused with an error, never a panic, and
//! the ASCII form reads like the binary one.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use kition_scene_fbx::node;

fn starter_file() -> Vec<u8> {
    kition_scene_fbx::write(&kition_scene_model::Scene::starter(), None).unwrap()
}

fn names(imported: kition_scene_exchange::Imported) -> Vec<String> {
    let mut scene = kition_scene_model::Scene::empty();
    kition_scene_exchange::place(&mut scene, imported).unwrap();
    let mut names: Vec<String> = scene.objects.iter().map(|object| object.name.clone()).collect();
    names.sort();
    names
}

#[test]
fn the_starter_scene_reads_back() {
    let names = names(kition_scene_fbx::read(&starter_file()).unwrap());
    assert_eq!(names.len(), 3, "{names:?}");
}

#[test]
fn the_ascii_form_reads_like_the_binary_one() {
    let bytes = starter_file();
    let (nodes, _) = node::parse(&bytes).unwrap();
    let ascii = node::write_ascii(&nodes);
    assert!(!node::is_binary(ascii.as_bytes()));
    assert_eq!(names(kition_scene_fbx::read(ascii.as_bytes()).unwrap()), names(kition_scene_fbx::read(&bytes).unwrap()));
}

#[test]
fn truncated_files_are_refused_without_a_panic() {
    let bytes = starter_file();
    for end in (0..bytes.len()).step_by(7) {
        let _ = kition_scene_fbx::read(&bytes[..end]);
    }
    let ascii = node::write_ascii(&node::parse(&bytes).unwrap().0);
    for end in (0..ascii.len()).step_by(5) {
        let _ = kition_scene_fbx::read(&ascii.as_bytes()[..end]);
    }
}

#[test]
fn corrupted_bytes_never_panic() {
    let bytes = starter_file();
    let mut state = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..2000 {
        let mut damaged = bytes.clone();
        for _ in 0..1 + (state % 4) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = (state % damaged.len() as u64) as usize;
            damaged[at] = (state >> 32) as u8;
        }
        let _ = kition_scene_fbx::read(&damaged);
    }
    // The text form reaches the importer with damaged values more often than zlib-packed arrays do.
    let ascii = node::write_ascii(&node::parse(&bytes).unwrap().0).into_bytes();
    let alphabet = b"0123456789-.,:{}*\"aZ \n";
    for _ in 0..2000 {
        let mut damaged = ascii.clone();
        for _ in 0..1 + (state % 3) {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let at = (state % damaged.len() as u64) as usize;
            damaged[at] = alphabet[((state >> 40) % alphabet.len() as u64) as usize];
        }
        let _ = kition_scene_fbx::read(&damaged);
    }
}

#[test]
fn absurd_sizes_are_refused() {
    let mut file = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
    file.extend_from_slice(&7400u32.to_le_bytes());
    // A node whose end offset points past the file and claims billions of properties.
    file.extend_from_slice(&u32::MAX.to_le_bytes());
    file.extend_from_slice(&u32::MAX.to_le_bytes());
    file.extend_from_slice(&u32::MAX.to_le_bytes());
    file.push(1);
    file.push(b'X');
    assert!(kition_scene_fbx::read(&file).is_err());
    // An array that claims a huge zlib payload.
    let mut file = b"Kaydara FBX Binary  \0\x1a\0".to_vec();
    file.extend_from_slice(&7400u32.to_le_bytes());
    let prop = [&b"d"[..], &u32::MAX.to_le_bytes(), &1u32.to_le_bytes(), &u32::MAX.to_le_bytes()].concat();
    file.extend_from_slice(&((27 + 1 + prop.len()) as u32).to_le_bytes());
    file.extend_from_slice(&1u32.to_le_bytes());
    file.extend_from_slice(&(prop.len() as u32).to_le_bytes());
    file.push(1);
    file.push(b'A');
    file.extend_from_slice(&prop);
    assert!(kition_scene_fbx::read(&file).is_err());
    // ASCII nesting deeper than any real file.
    let deep = format!("{}{}", "A: {\n".repeat(10_000), "}\n".repeat(10_000));
    assert!(kition_scene_fbx::read(deep.as_bytes()).is_err());
    let _ = kition_scene_fbx::read(b"A: *999999999999 { a: 1 }");
}

#[test]
fn cyclic_connections_are_survived() {
    let file = br#"; FBX 7.4.0 project file
Objects:  {
    Model: 1, "Model::A", "Null" {
    }
    Model: 2, "Model::B", "LimbNode" {
    }
    Model: 3, "Model::C", "LimbNode" {
    }
    Geometry: 4, "Geometry::G", "Mesh" {
        Vertices: *3 { a: 0,0,0 }
        PolygonVertexIndex: *3 { a: 0,0,-1 }
    }
}
Connections:  {
    C: "OO",1,2
    C: "OO",2,1
    C: "OO",2,3
    C: "OO",3,2
    C: "OO",4,4
    C: "OO",4,1
}
"#;
    let _ = kition_scene_fbx::read(file);
}
