use anyhow::{Context, Result};
use entities::{
    prelude::*,
    sea_orm_active_enums::{ImageKind, LocationFilter},
    *,
};
use migration::{Migrator, MigratorTrait};
use sea_orm::{Database as SeaDatabase, DatabaseConnection, *};
use sea_query::*;
use tracing::{instrument, log::LevelFilter};

pub struct Database {
    conn: DatabaseConnection,
}

impl Database {
    pub async fn new() -> Result<Self> {
        let db_url = std::env::var("DATABASE_URL")?;

        let mut conn_options = ConnectOptions::new(db_url);
        conn_options.sqlx_logging_level(LevelFilter::Debug);
        conn_options.sqlx_logging(true);

        let conn = SeaDatabase::connect(conn_options).await?;
        Migrator::up(&conn, None).await?;
        Ok(Self { conn })
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn create_or_update_user(
        &self,
        profile: crate::EditProfile,
    ) -> Result<()> {
        let id = profile.id;
        let user = profile.as_active_model();

        if Users::find_by_id(id).one(&self.conn).await?.is_some() {
            Users::update(user).exec(&self.conn).await?;
        } else {
            Users::insert(user).exec(&self.conn).await?;
        }

        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn get_images(
        &self,
        user_id: i64,
    ) -> Result<Vec<(String, ImageKind)>> {
        #[derive(FromQueryResult)]
        struct ImageTelegramId {
            telegram_id: String,
            kind: ImageKind,
        }
        Ok(images::Entity::find()
            .filter(images::Column::UserId.eq(user_id))
            .select_only()
            .columns([images::Column::TelegramId, images::Column::Kind])
            .into_model::<ImageTelegramId>()
            .all(&self.conn)
            .await?
            .into_iter()
            .map(|m| (m.telegram_id, m.kind))
            .collect())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn create_image(
        &self,
        user_id: i64,
        tg_id: String,
        kind: ImageKind,
    ) -> Result<()> {
        let image = entities::images::ActiveModel {
            user_id: ActiveValue::Set(user_id),
            telegram_id: ActiveValue::Set(tg_id),
            kind: ActiveValue::Set(kind),
            ..Default::default()
        };
        Images::insert(image).exec(&self.conn).await?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn create_state(&self, id: i64) -> Result<()> {
        if States::find_by_id(id).one(&self.conn).await?.is_none() {
            let state =
                entities::states::ActiveModel { id: ActiveValue::Set(id) };
            States::insert(state).exec(&self.conn).await?;
        }
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn clean_images(&self, user_id: i64) -> Result<()> {
        Images::delete_many()
            .filter(images::Column::UserId.eq(user_id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn get_user(&self, id: i64) -> Result<Option<users::Model>> {
        Ok(Users::find_by_id(id).one(&self.conn).await?)
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn get_dating(&self, id: i32) -> Result<datings::Model> {
        Datings::find_by_id(id)
            .one(&self.conn)
            .await?
            .context("dating not found")
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn update_last_activity(&self, id: i64) -> Result<()> {
        Users::update_many()
            .col_expr(
                users::Column::LastActivity,
                Expr::current_timestamp().into(),
            )
            .filter(users::Column::Id.eq(id))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn get_partner(
        &self,
        user_id: i64,
    ) -> Result<Option<(datings::Model, users::Model)>> {
        // Load dating initiator
        let user = Users::find_by_id(user_id)
            .one(&self.conn)
            .await?
            .context("user not found")?;

        self.update_last_activity(user_id).await?;

        // TODO: fix this
        let user_id_clone = user.id;

        let last_unresponded_dating = Datings::find()
            .filter(datings::Column::InitiatorId.eq(user_id))
            .filter(datings::Column::InitiatorReaction.is_null())
            .one(&self.conn)
            .await?;

        if let Some(dating) = last_unresponded_dating {
            let partner = Users::find_by_id(dating.partner_id)
                .one(&self.conn)
                .await?
                .context("partner not found")?;
            return Ok(Some((dating, partner)));
        }

        let mut partner_query = Users::find()
            // Don't recommend user to himself
            .filter(users::Column::Id.ne(user_id))
            // Only recommend activated profiles
            .filter(users::Column::Active.eq(true))
            // Only recommend active users
            .filter(users::Column::LastActivity.into_expr().gt(
                Expr::current_timestamp().sub(Expr::cust("interval '90 days'")),
            ))
            // Respect users's graduation delta preference
            .filter(users::Column::GraduationYear.between(
                user.graduation_year - user.grade_up_filter,
                user.graduation_year + user.grade_down_filter,
            ))
            // Respect partner's graduation delta preference
            .filter(
                users::Column::GraduationYear
                    .into_expr()
                    .add(users::Column::GradeDownFilter.into_expr())
                    .binary(
                        BinOper::GreaterThanOrEqual,
                        Expr::value(user.graduation_year),
                    )
                    .and(
                        users::Column::GraduationYear
                            .into_expr()
                            .sub(users::Column::GradeUpFilter.into_expr())
                            .binary(
                                BinOper::SmallerThanOrEqual,
                                Expr::value(user.graduation_year),
                            ),
                    ),
            )
            // Respect dating purpose
            .filter(
                Expr::cust_with_exprs("$1 & $2", [
                    users::Column::DatingPurpose
                        .into_expr()
                        .cast_as(Alias::new("integer"))
                        .cast_as(Alias::new("bit(16)")),
                    Expr::value(user.dating_purpose)
                        .cast_as(Alias::new("integer"))
                        .cast_as(Alias::new("bit(16)")),
                ])
                .ne(Expr::value(0i16)
                    .cast_as(Alias::new("integer"))
                    .cast_as(Alias::new("bit(16)"))),
            )
            // Respect partner's subject preference
            .filter(
                Condition::any()
                    .add(
                        Expr::cust_with_exprs("$1 & $2", [
                            users::Column::SubjectsFilter
                                .into_expr()
                                .cast_as(Alias::new("bit(32)")),
                            Expr::value(user.subjects)
                                .cast_as(Alias::new("bit(32)")),
                        ])
                        .ne(Expr::value(0i32).cast_as(Alias::new("bit(32)"))),
                    )
                    .add(users::Column::SubjectsFilter.eq(0i32)),
            )
            // Respect partner's gender preference
            .filter(
                Condition::any()
                    .add(users::Column::GenderFilter.is_null())
                    .add(
                        users::Column::GenderFilter
                            .eq(Some(user.gender.clone())),
                    ),
            )
            // Respect partner's location filter
            .filter(
                Condition::any()
                    // SameCountry
                    .add(
                        users::Column::LocationFilter
                            .eq(LocationFilter::SameCountry),
                    )
                    // SameCounty
                    .add(
                        Condition::all()
                            .add(
                                users::Column::LocationFilter
                                    .eq(LocationFilter::SameCounty),
                            )
                            .add(
                                users::Column::City
                                    .into_expr()
                                    .binary(BinOper::RShift, 16)
                                    .eq(user.city.unwrap_or(0) >> 16),
                            ),
                    )
                    // SameSubject
                    .add(
                        Condition::all()
                            .add(
                                users::Column::LocationFilter
                                    .eq(LocationFilter::SameSubject),
                            )
                            .add(
                                users::Column::City
                                    .into_expr()
                                    .binary(BinOper::RShift, 8)
                                    .binary(BinOper::Mod, 2i32.pow(8))
                                    .eq((user.city.unwrap_or(0) >> 8)
                                        % 2i32.pow(8)),
                            ),
                    )
                    // SameCity
                    .add(users::Column::City.eq(user.city)),
            )
            // Don't recommend the same partner more than once a week
            .join_rev(
                JoinType::LeftJoin,
                datings::Entity::belongs_to(users::Entity)
                    .from(datings::Column::PartnerId)
                    .to(users::Column::Id)
                    .on_condition(move |_left, _right| {
                        Condition::all()
                            .add(datings::Column::InitiatorId.eq(user_id_clone))
                            .add(
                                Condition::any()
                                    .add(datings::Column::Time.into_expr().gt(
                                        Expr::current_timestamp().sub(
                                            Expr::cust("interval '4 hours'"),
                                        ),
                                    ))
                                    .add(datings::Column::Time.into_expr().gt(
                                        Expr::current_timestamp().sub(
                                            Expr::cust("interval '7 days'"),
                                        ),
                                    ).and(datings::Column::InitiatorReaction.eq(true))),
                            )
                    })
                    .into(),
            )
            .group_by(users::Column::Id)
            .having(datings::Column::Id.count().eq(0))
            // Get random partner
            .order_by_desc(SimpleExpr::from(Func::random()));

        // Respect user's subject preference
        if user.subjects_filter != 0 {
            partner_query = partner_query.filter(
                Expr::cust_with_exprs("$1 & $2", [
                    users::Column::Subjects
                        .into_expr()
                        .cast_as(Alias::new("bit(32)")),
                    Expr::value(user.subjects_filter)
                        .cast_as(Alias::new("bit(32)")),
                ])
                .ne(Expr::value(0i32).cast_as(Alias::new("bit(32)"))),
            );
        }

        // Respect user's gender preference
        if let Some(g) = &user.gender_filter {
            partner_query =
                partner_query.filter(users::Column::Gender.eq(Some(g.clone())));
        }

        // Respect user's location filter
        partner_query = match user.location_filter {
            LocationFilter::SameCountry => partner_query, // Just match
            // everything
            LocationFilter::SameCounty => partner_query.filter(
                users::Column::City
                    .into_expr()
                    .binary(BinOper::RShift, 16)
                    .eq(user.city.context("user city must be set")? >> 16),
            ),
            LocationFilter::SameSubject => partner_query.filter(
                users::Column::City
                    .into_expr()
                    .binary(BinOper::RShift, 8)
                    .binary(BinOper::Mod, 2i32.pow(8))
                    .eq((user.city.context("user city must be set")? >> 8)
                        % 2i32.pow(8)),
            ),
            LocationFilter::SameCity => {
                partner_query.filter(users::Column::City.eq(user.city))
            }
        };

        let txn = self.conn.begin().await?;

        // println!("{}", partner_query.build(DatabaseBackend::Postgres));
        let partner = partner_query.one(&txn).await?;

        match partner {
            Some(p) => {
                // Save dating
                let dating = datings::ActiveModel {
                    initiator_id: ActiveValue::Set(user_id),
                    partner_id: ActiveValue::Set(p.id),
                    ..Default::default()
                };
                let dating_id =
                    Datings::insert(dating).exec(&txn).await?.last_insert_id;

                // TODO: 1 query
                let saved_dating: datings::Model =
                    Datings::find_by_id(dating_id)
                        .one(&txn)
                        .await?
                        .context("dating not found")?;

                txn.commit().await?;

                Ok(Some((saved_dating, p)))
            }
            None => Ok(None),
        }
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn set_dating_initiator_reaction(
        &self,
        dating: i32,
        reaction: bool,
    ) -> Result<()> {
        Datings::update_many()
            .filter(datings::Column::Id.eq(dating))
            .col_expr(datings::Column::InitiatorReaction, Expr::value(reaction))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn set_dating_partner_reaction(
        &self,
        dating: i32,
        reaction: bool,
    ) -> Result<()> {
        Datings::update_many()
            .filter(datings::Column::Id.eq(dating))
            .col_expr(datings::Column::PartnerReaction, Expr::value(reaction))
            .exec(&self.conn)
            .await?;
        Ok(())
    }

    #[instrument(level = "debug", skip(self))]
    pub async fn set_dating_initiator_msg(
        &self,
        dating: i32,
        msg: i32,
    ) -> Result<()> {
        Datings::update_many()
            .filter(datings::Column::Id.eq(dating))
            .col_expr(datings::Column::InitiatorMsgId, Expr::value(msg))
            .exec(&self.conn)
            .await?;
        Ok(())
    }
}
