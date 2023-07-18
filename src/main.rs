#![warn(clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::wildcard_imports,
    clippy::enum_glob_use,
    clippy::too_many_lines,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]

use std::{str::FromStr, sync::Arc};

use db::Database;
use sentry_tracing::EventFilter;
use teloxide::{
    adaptors::{throttle::Limits, Throttle},
    dispatching::dialogue::InMemStorage,
    error_handlers::ErrorHandler,
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup},
    utils::command::BotCommands,
    RequestError,
};
use tracing::*;
use tracing_subscriber::prelude::*;
use types::UserSettings;

mod callbacks;
mod cities;
mod datings;
mod db;
mod handle;
mod request;
mod text;
mod types;
mod utils;

type Bot = Throttle<teloxide::Bot>;
type MyDialogue = Dialogue<State, InMemStorage<State>>;

#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error(transparent)]
    Telegram(#[from] RequestError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

struct AppErrorHandler {}

impl AppErrorHandler {
    fn new() -> Arc<Self> {
        Arc::new(Self {})
    }
}

impl ErrorHandler<anyhow::Error> for AppErrorHandler {
    fn handle_error(
        self: Arc<Self>,
        error: anyhow::Error,
    ) -> futures_util::future::BoxFuture<'static, ()> {
        warn!("{}", error.to_string());
        sentry_anyhow::capture_anyhow(&error);

        Box::pin(async {})
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    std::env::set_var("RUST_BACKTRACE", "1"); // FIXME: HACK

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer().with_filter(
                tracing_subscriber::filter::LevelFilter::from_str(
                    &std::env::var("RUST_LOG")
                        .unwrap_or_else(|_| String::from("info")),
                )
                .unwrap_or(tracing_subscriber::filter::LevelFilter::INFO),
            ),
        )
        .with(sentry_tracing::layer().event_filter(|md| match *md.level() {
            Level::TRACE => EventFilter::Ignore,
            Level::ERROR => EventFilter::Event,
            _ => EventFilter::Breadcrumb,
        }))
        .try_init()
        .unwrap();

    let _sentry_guard = match std::env::var("SENTRY_DSN") {
        Ok(d) => {
            let guard = sentry::init((d, sentry::ClientOptions {
                release: sentry::release_name!(),
                default_integrations: true,
                attach_stacktrace: true,
                traces_sample_rate: 1.0,
                enable_profiling: true,
                profiles_sample_rate: 1.0,
                ..Default::default()
            }));
            Some(guard)
        }
        Err(e) => {
            warn!("can't get SENTRY_DSN: {:?}", e);
            None
        }
    };

    tracing::info!("Starting bot...");
    let bot = teloxide::Bot::from_env().throttle(Limits {
        messages_per_sec_chat: 2,
        messages_per_min_chat: 120,
        ..Default::default()
    });

    let handler = dptree::entry()
        .enter_dialogue::<Update, InMemStorage<State>, State>()
        // .branch(
        //     dptree::filter_map(|update: Update| {
        //         Some(!update.chat()?.is_private())
        //     })
        //     .endpoint(handle_public_chat),
        // )
        .branch(
            Update::filter_message()
                .branch(
                    dptree::entry()
                        .filter_command::<Command>()
                        .endpoint(answer),
                )
                .branch(dptree::endpoint(handle::handle_message)),
        )
        .branch(
            Update::filter_callback_query()
                .branch(dptree::endpoint(handle::handle_callback)),
        );

    let database = db::Database::new().await?;

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![
            InMemStorage::<State>::new(),
            Arc::new(database)
        ])
        .error_handler(AppErrorHandler::new())
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
    Ok(())
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct StateData {
    s: UserSettings,
    create_new: bool,
    photos_count: u8,
}

impl StateData {
    pub fn with_settings(s: UserSettings) -> Self {
        Self { s, ..Default::default() }
    }
}

#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub enum State {
    #[default]
    Start,
    // edit profile
    SetName(StateData),
    SetGender(StateData),
    SetGenderFilter(StateData),
    SetGraduationYear(StateData),
    SetSubjects(StateData),
    SetSubjectsFilter(StateData),
    SetDatingPurpose(StateData),
    SetCity(StateData),
    SetLocationFilter(StateData),
    SetAbout(StateData),
    SetPhotos(StateData),
    /// Waiting for the message for the like
    LikeWithMessage {
        dating: entities::datings::Model,
    },
    Edit,
}

#[derive(Debug, BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Доступные команды:")]
enum Command {
    #[command(description = "заполнить анкету")]
    Create,
    #[command(description = "показать мою анкету")]
    Profile,
    #[command(description = "изменить анкету")]
    Edit,
    #[command(description = "найти партнёра")]
    Date,
    #[command(description = "включить анкету")]
    Enable,
    #[command(description = "выключить анкету")]
    Disable,
    #[command(description = "приветственное сообщение")]
    Start,
    #[command(description = "помощь по командам")]
    Help,
}

pub async fn start_profile_creation(
    state: &mut State,
    msg: &Message,
    bot: &Bot,
) -> anyhow::Result<()> {
    let chat = &msg.chat;
    handle::make_macros!(bot, msg, state, chat);

    remove_buttons!();
    if !utils::check_user_subscribed_channel(bot, msg.chat.id.0).await? {
        send!(
            text::SUBSCRIBE_TEXT,
            inline[[InlineKeyboardButton::callback(
                "Я подписался на канал",
                "✍",
            )]]
        );
        return Ok(());
    };

    if utils::user_url(bot, msg.chat.id.0).await?.is_none() {
        send!(
            text::PLEASE_ALLOW_FORWARDING,
            inline[[InlineKeyboardButton::callback("Я сделал юзернейм", "✍",)]]
        );
    } else {
        send!(text::PROFILE_CREATION_STARTED);
        let settings = UserSettings::with_id(msg.chat.id.0);
        upd_print!(State::SetName(StateData::with_settings(settings)));
    }

    Ok(())
}

#[tracing::instrument(err, skip(db, bot))]
async fn answer(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    state: State,
    msg: Message,
    cmd: Command,
) -> anyhow::Result<()> {
    async fn inner(
        db: Arc<Database>,
        bot: Bot,
        dialogue: MyDialogue,
        mut state: State,
        msg: Message,
        cmd: Command,
    ) -> anyhow::Result<()> {
        match cmd {
            Command::Create => {
                start_profile_creation(&mut state, &msg, &bot).await?;
                dialogue.update(state).await?;
            }
            Command::Edit => {
                if db.get_user(msg.chat.id.0).await?.is_none() {
                    bot.send_message(msg.chat.id, text::PLEASE_CREATE_PROFILE)
                        .await?;
                    return Ok(());
                }

                request::edit_profile(&bot, &msg.chat).await?;
                dialogue.update(State::Edit).await?;
            }
            Command::Help => {
                bot.send_message(
                    msg.chat.id,
                    Command::descriptions().to_string(),
                )
                .await?;
            }
            Command::Date => {
                if db.get_user(msg.chat.id.0).await?.is_none() {
                    bot.send_message(msg.chat.id, text::PLEASE_CREATE_PROFILE)
                        .await?;
                    return Ok(());
                }

                datings::send_recommendation(&bot, &db, msg.chat.id).await?;
            }
            Command::Profile => {
                if db.get_user(msg.chat.id.0).await?.is_none() {
                    bot.send_message(msg.chat.id, text::PLEASE_CREATE_PROFILE)
                        .await?;
                    return Ok(());
                }

                datings::send_profile(&bot, &db, msg.chat.id.0).await?;
            }
            Command::Enable => {
                if db.get_user(msg.chat.id.0).await?.is_none() {
                    bot.send_message(msg.chat.id, text::PLEASE_CREATE_PROFILE)
                        .await?;
                    return Ok(());
                }

                db.create_or_update_user(UserSettings {
                    active: Some(true),
                    ..UserSettings::with_id(msg.chat.id.0)
                })
                .await?;
                bot.send_message(msg.chat.id, text::PROFILE_ENABLED).await?;
            }
            Command::Disable => {
                if db.get_user(msg.chat.id.0).await?.is_none() {
                    bot.send_message(msg.chat.id, text::PLEASE_CREATE_PROFILE)
                        .await?;
                    return Ok(());
                }

                db.create_or_update_user(UserSettings {
                    active: Some(false),
                    ..UserSettings::with_id(msg.chat.id.0)
                })
                .await?;
                bot.send_message(msg.chat.id, text::PROFILE_DISABLED).await?;
            }
            Command::Start => {
                db.create_state(msg.chat.id.0).await?;

                let keyboard = vec![vec![InlineKeyboardButton::callback(
                    "Заполнить анкету ✍",
                    "✍",
                )]];
                let keyboard_markup = InlineKeyboardMarkup::new(keyboard);

                bot.send_message(msg.chat.id, text::START)
                    .reply_markup(keyboard_markup)
                    .await?;
            }
        }

        Ok(())
    }
    // FIXME: remove this
    if let Err(e) =
        inner(db, bot.clone(), dialogue, state, msg.clone(), cmd).await
    {
        bot.send_message(
            msg.chat.id,
            format!("АаААА, ошибка стоп 000000: {e}"),
        )
        .await?;
        return Err(e);
    }

    Ok(())
}
