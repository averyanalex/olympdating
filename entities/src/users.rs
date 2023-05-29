//! `SeaORM` Entity. Generated by sea-orm-codegen 0.11.3

use super::sea_orm_active_enums::Gender;
use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel, Eq)]
#[sea_orm(table_name = "users")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: i64,
    pub name: String,
    pub gender: Gender,
    pub gender_pref: Option<Gender>,
    pub about: String,
    pub active: bool,
    pub last_activity: DateTime,
    pub graduation_year: i16,
    pub up_graduation_year_delta_pref: i16,
    pub down_graduation_year_delta_pref: i16,
    pub subjects: i64,
    pub subjects_prefs: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {
    #[sea_orm(has_many = "super::images::Entity")]
    Images,
}

impl Related<super::images::Entity> for Entity {
    fn to() -> RelationDef {
        Relation::Images.def()
    }
}

impl ActiveModelBehavior for ActiveModel {}
