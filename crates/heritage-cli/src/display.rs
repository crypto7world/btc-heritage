pub trait Displayable {
    fn display(&self);
}

impl<T: serde::Serialize> Displayable for T {
    fn display(&self) {
        println!(
            "{}",
            serde_json::to_string_pretty(self)
                .expect("Caller responsability to ensure Json serialization works")
        )
    }
}
