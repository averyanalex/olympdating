use std::{fmt::Display, str::FromStr};

use anyhow::Context;
use itertools::Itertools;
use strsim::jaro_winkler;

include!(concat!(env!("OUT_DIR"), "/citiesmap.rs"));

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct UserCity(Option<City>);

impl Display for UserCity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(city) => {
                f.write_fmt(format_args!("{city}"))?;
            }
            None => f.write_str("Город не указан")?,
        }

        Ok(())
    }
}

impl UserCity {
    pub const fn get_city(self) -> Option<City> {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct City(i32);

impl City {
    pub fn city(&self) -> &'static &'static str {
        city_by_id(self.0).expect("city not found")
    }

    pub fn subject(&self) -> &'static &'static str {
        subject_by_id(self.0).expect("subject not found")
    }

    pub fn county(&self) -> &'static &'static str {
        county_by_id(self.0).expect("county not found")
    }
}

impl TryFrom<i32> for City {
    type Error = anyhow::Error;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        county_by_id(value).context("county not found")?;
        subject_by_id(value).context("subject not found")?;
        city_by_id(value).context("city not found")?;

        Ok(Self(value))
    }
}

impl Display for City {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let county = self.county();
        let subject = self.subject();
        let city = self.city();

        if subject == city {
            f.write_fmt(format_args!("{county} ФО, {city}"))?;
        } else {
            f.write_fmt(format_args!("{county} ФО, {subject}, {city}"))?;
        };
        Ok(())
    }
}

impl FromStr for UserCity {
    type Err = ();

    fn from_str(query: &str) -> Result<Self, Self::Err> {
        let best_city = CITIES
            .entries()
            .sorted_unstable_by(|(_, left), (_, right)| {
                jaro_winkler(&query.to_lowercase(), &left.to_lowercase())
                    .total_cmp(&jaro_winkler(
                        &query.to_lowercase(),
                        &right.to_lowercase(),
                    ))
            })
            .next_back()
            .expect("there must be at least 1 city");
        if jaro_winkler(best_city.1, query) > 0.15 {
            Ok(Self(Some(City(*best_city.0))))
        } else {
            Err(())
        }
    }
}

impl UserCity {
    pub const fn unspecified() -> Self {
        Self(None)
    }
}

impl TryFrom<Option<i32>> for UserCity {
    type Error = anyhow::Error;

    fn try_from(value: Option<i32>) -> Result<Self, Self::Error> {
        let city = match value {
            Some(id) => Self(Some(id.try_into()?)),
            None => Self(None),
        };
        Ok(city)
    }
}

impl From<UserCity> for Option<i32> {
    fn from(value: UserCity) -> Self {
        value.0.map(|v| v.0)
    }
}

pub fn county_by_id(id: i32) -> Option<&'static &'static str> {
    COUNTIES.get(&(id >> 16))
}

pub fn subject_by_id(id: i32) -> Option<&'static &'static str> {
    SUBJECTS.get(&((id >> 8) % 2i32.pow(8)))
}

pub fn city_by_id(id: i32) -> Option<&'static &'static str> {
    CITIES.get(&id)
}

pub fn county_exists(name: &str) -> bool {
    COUNTIES_REV.get(name).is_some()
}

pub fn subject_exists(name: &str) -> bool {
    SUBJECTS_REV.get(name).is_some()
}

pub fn city_exists(name: &str) -> bool {
    CITIES_REV.get(name).is_some()
}
