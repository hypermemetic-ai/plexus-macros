//! What `plexus_macros::Deserialize` is not for, each named.

#[derive(plexus_macros::Deserialize)]
pub struct Said {
    pub text: String,
}

#[derive(plexus_macros::Deserialize)]
pub enum Untagged {
    Said { text: String },
}

#[derive(plexus_macros::Deserialize)]
#[serde(tag = "t", content = "c")]
pub enum Adjacent {
    Said { text: String },
}

#[derive(plexus_macros::Deserialize)]
#[serde(untagged)]
pub enum Bare {
    Said { text: String },
}

#[derive(plexus_macros::Deserialize)]
#[serde(tag = "type")]
pub enum Generic<T> {
    Said { text: T },
}

#[derive(plexus_macros::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Mixed {
    Said { text: String },
    #[serde(untagged)]
    Anything(String),
}

fn main() {}
