fn load_config(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|error| error.to_string())
}

fn save_config(path: &str, value: &str) -> Result<(), String> {
    std::fs::write(path, value).map_err(|error| error.to_string())
}

struct Service {
    name: String,
}

impl Service {
    fn name(&self) -> &str {
        &self.name
    }
}
