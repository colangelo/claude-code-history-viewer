//! `HubConfig::load` runs `validate()` (security C3, openspec/changes/read-only-tokens rev 2):
//! a file with a duplicate token across `[[tokens]]` and `[[readers]]` must not load, and the
//! same file without the duplicate must. One test in its own binary, because it sets
//! `HUB_CONFIG` for the process.

use uuid::Uuid;

#[test]
fn load_refuses_a_duplicate_and_accepts_the_fixed_file() {
    let dir = std::env::temp_dir().join(format!("cchv-load-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hub.toml");
    let file = |reader_token: &str| {
        format!(
            "database_url = \"postgres://unused\"\n\
             [[tokens]]\ntoken = \"secret-guest\"\nmachine_id = \"{}\"\nlabel = \"guest\"\ningest_only = true\n\
             [[readers]]\ntoken = \"{reader_token}\"\nmachine_id = \"{}\"\nlabel = \"blog\"\n",
            Uuid::new_v4(),
            Uuid::new_v4()
        )
    };
    // The only test in this binary: nothing else reads the environment concurrently.
    std::env::set_var("HUB_CONFIG", &path);

    std::fs::write(&path, file("secret-guest")).unwrap();
    let err = hub::HubConfig::load().unwrap_err().to_string();
    assert!(err.contains("duplicate"), "{err}");
    assert!(!err.contains("secret-"), "the error must not print a token: {err}");

    std::fs::write(&path, file("secret-reader")).unwrap();
    let config = hub::HubConfig::load().expect("the fixed file loads");
    assert_eq!(config.readers.len(), 1);

    let _ = std::fs::remove_dir_all(&dir);
}
