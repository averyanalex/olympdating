use std::sync::Arc;

use anyhow::{bail, Context};
use db::Database;
use entities::sea_orm_active_enums::ImageKind;
use teloxide::{
    // net::Download,
    prelude::*,
    types::{
        Chat, InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton,
        KeyboardMarkup, KeyboardRemove,
    },
};
use tracing::instrument;

use crate::{
    cities::{self, City},
    db, text,
    types::{
        DatingPurpose, Grade, GraduationYear, LocationFilter, Subjects,
        UserGender, GenderFilter,
    },
    utils, Bot, MyDialogue, State, StateData,
};

#[instrument(level = "debug", skip(bot, db))]
pub async fn next_state(
    dialogue: &MyDialogue,
    chat: &Chat,
    state: &State,
    s: StateData,
    bot: &Bot,
    db: &Arc<Database>,
) -> anyhow::Result<()> {
    use State::*;
    let next_state = match state {
        SetName(StateData { create_new: true, .. }) => SetGender(s.clone()),
        SetGender(StateData { create_new: true, .. }) => {
            SetGenderFilter(s.clone())
        }
        SetGenderFilter(StateData { create_new: true, .. }) => {
            SetGraduationYear(s.clone())
        }
        SetGraduationYear(StateData { create_new: true, .. }) => {
            SetSubjects(s.clone())
        }
        SetSubjects(_) => SetSubjectsFilter(s.clone()),
        SetSubjectsFilter(StateData { create_new: true, .. }) => {
            SetDatingPurpose(s.clone())
        }
        SetDatingPurpose(StateData { create_new: true, .. }) => {
            SetCity(s.clone())
        }
        SetCity(_) => {
            if s.s
                .city
                .context("city must be set when city editing finished")?
                .0
                .is_some()
            {
                SetLocationFilter(s.clone())
            } else if s.create_new {
                SetAbout(s.clone())
            } else {
                Start
            }
        }
        SetLocationFilter(StateData { create_new: true, .. }) => {
            SetAbout(s.clone())
        }
        SetAbout(StateData { create_new: true, .. }) => {
            // HACK: create user before setting photos
            db.create_or_update_user(s.s.try_into()?).await?;
            SetPhotos(s.clone())
        }
        SetPhotos(_) => {
            crate::datings::send_profile(bot, db, s.s.id).await?;
            Start
        }
        // invalid states
        Start | LikeMessage { .. } | Edit => {
            dialogue.exit().await?;
            anyhow::bail!("wrong state: {:?}", state)
        }
        // *(EditProfile { create_new: true, .. })
        _ => {
            db.create_or_update_user(s.s.try_into()?).await?;
            crate::datings::send_profile(bot, db, s.s.id).await?;
            Start
        }
    };
    print_current_state(&next_state, Some(&s), bot, chat).await?;
    dialogue.update(next_state).await?;

    Ok(())
}

#[instrument(level = "debug", skip(bot))]
pub async fn print_current_state(
    state: &State,
    p: Option<&StateData>,
    bot: &Bot,
    chat: &Chat,
) -> anyhow::Result<()> {
    use State::*;

    use crate::request::*;

    match state {
        // edit profile
        SetName(_) => request_set_name(bot, chat).await?,
        SetGender(_) => request_set_gender(bot, chat).await?,
        SetGenderFilter(_) => request_set_gender_filter(bot, chat).await?,
        SetGraduationYear(_) => request_set_grade(bot, chat).await?,
        SetSubjects(_) => {
            request_set_subjects(
                bot,
                chat,
                p.context("profile must be provided")?,
            )
            .await?
        }
        SetSubjectsFilter(_) => {
            request_set_subjects_filter(
                bot,
                chat,
                p.context("profile must be provided")?,
            )
            .await?
        }
        SetDatingPurpose(_) => {
            request_set_dating_purpose(
                bot,
                chat,
                p.context("profile must be provided")?,
            )
            .await?
        }
        SetCity(_) => request_set_city(bot, chat).await?,
        SetLocationFilter(_) => {
            request_set_location_filter(
                bot,
                chat,
                p.context("profile must be provided")?,
            )
            .await?
        }
        SetAbout(_) => request_set_about(bot, chat).await?,
        SetPhotos(_) => request_set_photos(bot, chat).await?,
        // others
        LikeMessage { .. } => {
            crate::datings::request_like_msg(bot, chat).await?
        }
        Edit => request_edit_profile(bot, chat).await?,
        // invalid states
        Start => {}
    };
    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_city(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    let text = msg.text().context("no text in message")?;

    match text {
        "Верно" => {
            if s.s.city.is_none() {
                bail!("try to confirm not set city")
            }
            next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;
        }
        "Не указывать" => {
            s.s.city = Some(City(None));
            s.s.location_filter = Some(LocationFilter::Country);

            bot.send_message(msg.chat.id, text::NO_CITY)
                .reply_markup(KeyboardRemove::new())
                .await?;

            next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;
        }
        // "Список городов" => {
        //     let cities: String = crate::cities::cities_list();

        //     bot.send_message(msg.chat.id, cities).await?;
        // }
        _ => match text.parse::<City>() {
            Ok(city) => {
                s.s.city = Some(city);
                dialogue.update(State::SetCity(s)).await?;

                let keyboard = vec![vec![
                    KeyboardButton::new("Верно"),
                    KeyboardButton::new("Не указывать"),
                ]];
                let keyboard_markup =
                    KeyboardMarkup::new(keyboard).resize_keyboard(true);
                bot.send_message(msg.chat.id, format!("Ваш город - {city}?",))
                    .reply_markup(keyboard_markup)
                    .await?;
            }
            Err(_) => {
                let keyboard = vec![vec![KeyboardButton::new("Не указывать")]];
                let keyboard_markup =
                    KeyboardMarkup::new(keyboard).resize_keyboard(true);
                bot.send_message(msg.chat.id, text::CANT_FIND_CITY)
                    .reply_markup(keyboard_markup)
                    .await?;
            }
        },
    }

    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_location_filter(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    let text = msg.text().context("no text in message")?;

    let Ok(location_filter) = text.parse::<LocationFilter>() else {
        print_current_state(&state, Some(&s), &bot, &msg.chat).await?;
        return Ok(());
    };

    s.s.location_filter = Some(location_filter);
    next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;

    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_name(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    match msg.text() {
        Some(text) if (3..=16).contains(&text.chars().count()) => {
            s.s.name = Some(text.to_owned());
            next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;
        }
        _ => {
            print_current_state(&state, Some(&s), &bot, &msg.chat).await?;
        }
    }
    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_gender(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    let Ok(gender) = msg.text().context("no text in message")?.parse::<UserGender>() else {
        print_current_state(&state, Some(&s), &bot, &msg.chat).await?;
        return Ok(());
    };

    s.s.gender = Some(gender);
    next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;

    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_partner_gender(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    let Ok(gender_filter) = msg.text().context("no text in message")?.parse::<GenderFilter>() else {
        print_current_state(&state, Some(&s), &bot, &msg.chat).await?;
        return Ok(());
    };

    s.s.gender_filter = Some(gender_filter);
    next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;

    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_grade(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    let Ok(grade) = msg
        .text()
        .context("no text in message")?
        .parse::<i8>()
    else {
        print_current_state(&state, Some(&s), &bot, &msg.chat).await?;
        return Ok(())
    };

    let Ok(grade) = Grade::try_from(grade) else {
        print_current_state(&state, Some(&s), &bot, &msg.chat).await?;
        return Ok(());
    };

    s.s.grade = Some(grade);
    next_state(&dialogue, &msg.chat, &state, s, &bot, &db).await?;

    // bot.send_message(
    //     msg.chat.id,
    //     format!(
    //         "Хорошо, сейчас вы в {grade} классе и закончите школу в \
    //          {graduation_year} году.\nИзменить это можно командой /setgrade"
    //     ),
    // )
    // .reply_markup(KeyboardRemove::new())
    // .await?;
    // print_next_state(&state, bot, msg.chat).await?;

    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_about(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    match msg.text() {
        Some(text) if (1..=1024).contains(&text.chars().count()) => {
            s.s.about = Some(text.to_owned());
            next_state(&dialogue, &msg.chat, &state, s, &bot, &db)
                .await?;
        }
        _ => {
            print_current_state(&state, Some(&s), &bot, &msg.chat)
                .await?;
        }
    }
    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_set_photos(
    db: Arc<Database>,
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    mut s: StateData,
    state: State,
) -> anyhow::Result<()> {
    let keyboard = vec![vec![KeyboardButton::new("Сохранить")]];
    let keyboard_markup = KeyboardMarkup::new(keyboard).resize_keyboard(true);

    match msg.text() {
        Some(text) if text == "Без фото" => {
            db.clean_images(msg.chat.id.0).await?;
            next_state(&dialogue, &msg.chat, &state, s, &bot, &db)
                .await?;
            return Ok(());
        }
        Some(text) if text == "Сохранить" => {
            next_state(&dialogue, &msg.chat, &state, s, &bot, &db)
                .await?;
            return Ok(());
        }
        _ => {
            if s.photos_count == 0 {
                db.clean_images(msg.chat.id.0).await?;
            } else if s.photos_count >= 10 {
                bot.send_message(
                    msg.chat.id,
                    "Невозможно добавить более 10 фото/видео",
                )
                .reply_markup(keyboard_markup)
                .await?;
                return Ok(());
            };

            if let Some(photo_sizes) = msg.photo() {
                let photo = &photo_sizes[photo_sizes.len() - 1];
                let photo_file = bot.get_file(photo.file.clone().id).await?;

                db.create_image(
                    msg.chat.id.0,
                    photo_file.id.clone(),
                    ImageKind::Image,
                )
                .await?;
            } else if let Some(video) = msg.video() {
                let video_file = bot.get_file(video.file.clone().id).await?;

                db.create_image(
                    msg.chat.id.0,
                    video_file.id.clone(),
                    ImageKind::Video,
                )
                .await?;
            } else {
                print_current_state(&state, Some(&s), &bot, &msg.chat)
                    .await?;
                return Ok(())
            };
        }
    };

    s.photos_count += 1;

    bot.send_message(
        msg.chat.id,
        format!(
            "Добавлено {}/10 фото/видео. Добавить ещё?",
            s.photos_count
        ),
    )
    .reply_markup(keyboard_markup)
    .await?;

    dialogue.update(State::SetPhotos(s)).await?;

    Ok(())
}

#[instrument(level = "debug", skip(bot, db))]
pub async fn handle_callback(
    bot: Bot,
    db: Arc<Database>,
    dialogue: MyDialogue,
    state: State,
    q: CallbackQuery,
) -> anyhow::Result<()> {
    let data = q.data.context("callback data not provided")?;
    let msg = q.message.context("callback without message")?;

    let first_char = data.chars().next().context("first char not found")?;
    let last_chars = data.chars().skip(1).collect::<String>();

    // TODO: refactor
    fn get_profile(state: &State) -> anyhow::Result<StateData> {
        match state {
            State::SetSubjects(e) => Ok(e.clone()),
            State::SetSubjectsFilter(e) => Ok(e.clone()),
            State::SetDatingPurpose(e) => Ok(e.clone()),
            _ => bail!("failed to get state data from state"),
        }
    }

    match first_char {
        // Start profile creation
        '✍' => {
            bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;
            crate::start_profile_creation(&dialogue, &msg, &bot).await?;
        }
        // Find partner
        '🚀' => {
            bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;
            crate::datings::send_recommendation(&bot, &db, msg.chat.id).await?;
        }
        // Edit profile
        'e' => {
            use State::*;

            let user =
                db.get_user(msg.chat.id.0).await?.context("user not found")?;
            let p = EditProfile::from_model(user);
            let state = match last_chars.as_str() {
                "Имя" => SetName(p.clone()),
                "Предметы" => SetSubjects(p.clone()),
                "О себе" => SetAbout(p.clone()),
                "Город" => SetCity(p.clone()),
                "Фото" => SetPhotos(p.clone()),
                "Отмена" => {
                    bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;
                    dialogue.exit().await?;
                    return Ok(());
                }
                _ => {
                    bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;
                    crate::request::request_edit_profile(&bot, &msg.chat)
                        .await?;
                    return Ok(());
                }
            };

            bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;
            print_current_state(&state, Some(&p), &bot, &msg.chat).await?;
            dialogue.update(state).await?;
        }
        // Dating purpose
        'p' => {
            let mut profile = get_profile(&state)?;

            let purpose = match profile.dating_purpose {
                Some(s) => DatingPurpose::try_from(s)?,
                None => DatingPurpose::empty(),
            };

            if last_chars == "continue" {
                if purpose == DatingPurpose::empty() {
                    bail!("there must be at least 1 purpose")
                }

                bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;

                bot.edit_message_text(
                    msg.chat.id,
                    msg.id,
                    format!("Вас интересует: {purpose}.",),
                )
                .await?;

                profile.dating_purpose = Some(purpose.bits());
                next_state(&dialogue, &msg.chat, &state, profile, &bot, &db)
                    .await?;
            } else {
                let purpose = purpose
                    ^ DatingPurpose::from_bits(last_chars.parse()?)
                        .context("purpose error")?;

                bot.edit_message_reply_markup(msg.chat.id, msg.id)
                    .reply_markup(utils::make_dating_purpose_keyboard(purpose))
                    .await?;

                profile.dating_purpose = Some(purpose.bits());
                dialogue.update(State::SetDatingPurpose(profile)).await?;
            }
        }
        // Subjects
        's' => {
            let mut profile = get_profile(&state)?;

            let subjects = match profile.subjects {
                Some(s) => Subjects::from_bits(s)
                    .context("subjects must be created")?,
                None => Subjects::empty(),
            };

            if last_chars == "continue" {
                bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;

                let subjects_str = if subjects.is_empty() {
                    "Вы ничего не ботаете.".to_owned()
                } else {
                    format!("Предметы, которые вы ботаете: {subjects}.",)
                };
                bot.edit_message_text(
                    msg.chat.id,
                    msg.id,
                    format!("{subjects_str}",),
                )
                .await?;

                profile.subjects = Some(subjects.bits());
                next_state(&dialogue, &msg.chat, &state, profile, &bot, &db)
                    .await?;
            } else {
                let subjects = subjects
                    ^ Subjects::from_bits(last_chars.parse()?)
                        .context("subjects error")?;

                bot.edit_message_reply_markup(msg.chat.id, msg.id)
                    .reply_markup(utils::make_subjects_keyboard(
                        subjects,
                        utils::SubjectsKeyboardType::User,
                    ))
                    .await?;

                profile.subjects = Some(subjects.bits());
                dialogue.update(State::SetSubjects(profile)).await?;
            }
        }
        // Subjects filter
        'd' => {
            let mut profile = get_profile(&state)?;

            let subjects_filter = match profile.subjects_filter {
                Some(s) => Subjects::from_bits(s)
                    .context("subjects must be created")?,
                None => Subjects::empty(),
            };

            if last_chars == "continue" {
                bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;

                let subjects_filter_str = if subjects_filter.is_empty() {
                    "Не важно, что ботает другой человек.".to_owned()
                } else {
                    format!(
                        "Предметы, хотя бы один из которых должен ботать тот, \
                         кого вы ищете: {subjects_filter}.",
                    )
                };
                bot.edit_message_text(
                    msg.chat.id,
                    msg.id,
                    format!("{subjects_filter_str}",),
                )
                .await?;

                profile.subjects_filter = Some(subjects_filter.bits());
                next_state(&dialogue, &msg.chat, &state, profile, &bot, &db)
                    .await?;
            } else {
                let subjects_filter = subjects_filter
                    ^ Subjects::from_bits(last_chars.parse()?)
                        .context("subjects error")?;

                bot.edit_message_reply_markup(msg.chat.id, msg.id)
                    .reply_markup(utils::make_subjects_keyboard(
                        subjects_filter,
                        utils::SubjectsKeyboardType::Partner,
                    ))
                    .await?;

                profile.subjects_filter = Some(subjects_filter.bits());
                dialogue.update(State::SetSubjectsFilter(profile)).await?;
            }
        }
        // Dating response callbacks
        '👎' | '💌' | '👍' | '💔' | '❤' => {
            let id = last_chars.parse()?;
            let dating = db.get_dating(id).await?;

            match first_char {
                '👎' => {
                    bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;

                    if dating.initiator_reaction.is_some() {
                        bail!("user abuses dislikes")
                    }

                    db.set_dating_initiator_reaction(id, false).await?;
                    crate::datings::send_recommendation(
                        &bot,
                        &db,
                        ChatId(dating.initiator_id),
                    )
                    .await?;
                }
                '💌' => {
                    bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;

                    if dating.initiator_reaction.is_some() {
                        bail!("user abuses msglikes")
                    }

                    let state = State::LikeMessage { dating };
                    crate::handle::print_current_state(
                        &state, None, &bot, &msg.chat,
                    )
                    .await?;
                    dialogue.update(state).await?;
                }
                '👍' => {
                    bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;

                    if dating.initiator_reaction.is_some() {
                        bail!("user abuses likes")
                    }

                    db.set_dating_initiator_reaction(id, true).await?;
                    crate::datings::send_recommendation(
                        &bot,
                        &db,
                        ChatId(dating.initiator_id),
                    )
                    .await?;
                    crate::datings::send_like(&db, &bot, &dating, None).await?;
                }
                '💔' => {
                    if dating.partner_reaction.is_some() {
                        bail!("partner abuses dislikes")
                    }

                    bot.edit_message_reply_markup(msg.chat.id, msg.id).await?;
                    db.set_dating_partner_reaction(id, false).await?
                }
                '❤' => {
                    if dating.partner_reaction.is_some() {
                        bail!("partner abuses likes")
                    }

                    let initiator = db
                        .get_user(dating.initiator_id)
                        .await?
                        .context("dating initiator not found")?;

                    let partner_keyboard =
                        vec![vec![InlineKeyboardButton::url(
                            "Открыть чат",
                            crate::utils::user_url(&bot, initiator.id)
                                .await?
                                .context("can't get url")?,
                        )]];
                    let partner_keyboard_markup =
                        InlineKeyboardMarkup::new(partner_keyboard);
                    if let Err(e) = bot
                        .edit_message_reply_markup(msg.chat.id, msg.id)
                        .reply_markup(partner_keyboard_markup)
                        .await
                    {
                        sentry_anyhow::capture_anyhow(
                            &anyhow::Error::from(e).context(
                                "error editing mutual like partner's message",
                            ),
                        );
                    }

                    crate::datings::mutual_like(&bot, &db, &dating).await?;
                }
                _ => bail!("unknown callback"),
            }
        }
        _ => bail!("unknown callback"),
    }

    Ok(())
}
