use std::{
    fmt::{Display, Formatter},
    str::FromStr,
};

use isahc::http::{uri::InvalidUri, Uri};

#[derive(Debug, Clone)]
pub struct Url(Uri);

impl Display for Url {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug)]
pub enum InvalidUrl {
    Uri(InvalidUri),
    NoScheme,
}

impl FromStr for Url {
    type Err = InvalidUrl;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uri = Uri::from_str(s).map_err(InvalidUrl::Uri)?;
        if uri.scheme_str().is_none() {
            return Err(InvalidUrl::NoScheme);
        }
        Ok(Url(uri))
    }
}

impl From<Url> for String {
    fn from(value: Url) -> Self {
        value.0.to_string()
    }
}

pub fn with_path(Url(uri): &Url, path: &str) -> Uri {
    isahc::http::uri::Builder::new()
        .scheme(uri.scheme_str().unwrap())
        .authority(uri.authority().unwrap().as_str())
        .path_and_query(path)
        .build()
        .unwrap()
}
