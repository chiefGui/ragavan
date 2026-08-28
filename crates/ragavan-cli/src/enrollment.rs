use crate::presentation::{HumanOutput, Response};
use ragavan_application::Enrollment;
use serde_json::{Map, Value as JsonValue};
use std::io;

impl Response for Enrollment {
    fn write_human(&self, output: &mut HumanOutput<'_>) -> io::Result<()> {
        output.success(format_args!(
            "{}",
            match self {
                Self::Enabled => "Ragavan is enabled for this repository.",
                Self::Disabled => "Ragavan is disabled for this repository.",
            }
        ))
    }

    fn json_object(&self) -> Map<String, JsonValue> {
        let state = match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        };
        Map::from_iter([("enrollment".to_owned(), JsonValue::from(state))])
    }
}
