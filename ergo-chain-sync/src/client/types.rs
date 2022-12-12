use std::fmt::{Display, Formatter};

#[derive(Debug, Clone)]
pub struct Url(String);

impl Display for Url {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<String> for Url {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for Url {
    fn from(value: &str) -> Self {
        Self(String::from(value))
    }
}

impl From<Url> for String {
    fn from(value: Url) -> Self {
        value.0
    }
}
