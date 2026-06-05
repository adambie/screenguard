pub fn notif_lock_title(lang: &str) -> &'static str {
    match lang {
        "es" => "Tiempo de pantalla agotado",
        "fr" => "Temps d'écran épuisé",
        "de" => "Bildschirmzeit abgelaufen",
        "pt" => "Tempo de ecrã esgotado",
        "pl" => "Czas ekranu minął",
        _    => "Screen time ended",
    }
}

pub fn notif_lock_body(lang: &str) -> &'static str {
    match lang {
        "es" => "La pantalla se bloqueará en 4 segundos.",
        "fr" => "L'écran sera verrouillé dans 4 secondes.",
        "de" => "Der Bildschirm wird in 4 Sekunden gesperrt.",
        "pt" => "O ecrã será bloqueado em 4 segundos.",
        "pl" => "Ekran zostanie zablokowany za 4 sekundy.",
        _    => "Your screen will be locked in 4 seconds.",
    }
}

pub fn notif_warning_title(lang: &str) -> &'static str {
    match lang {
        "es" => "Tiempo de pantalla",
        "fr" => "Temps d'écran",
        "de" => "Bildschirmzeit",
        "pt" => "Tempo de ecrã",
        "pl" => "Czas ekranu",
        _    => "Screen time",
    }
}

pub fn notif_warning_body(lang: &str, remaining: i32) -> String {
    let mins = plural_mins(lang, remaining);
    match lang {
        "es" => format!("Quedan {remaining} {mins} de pantalla."),
        "fr" => format!("Il reste {remaining} {mins} de temps d'écran."),
        "de" => format!("Noch {remaining} {mins} Bildschirmzeit."),
        "pt" => format!("Restam {remaining} {mins} de ecrã."),
        "pl" => format!("Pozostało {remaining} {mins} czasu ekranu."),
        _    => format!("{remaining} {mins} of screen time remaining."),
    }
}

pub fn notif_schedule_title(lang: &str) -> &'static str {
    match lang {
        "es" => "Tiempo de pantalla",
        "fr" => "Temps d'écran",
        "de" => "Bildschirmzeit",
        "pt" => "Tempo de ecrã",
        "pl" => "Czas ekranu",
        _    => "Screen time",
    }
}

pub fn notif_schedule_updated(lang: &str) -> &'static str {
    match lang {
        "es" => "El horario de pantalla ha sido actualizado.",
        "fr" => "L'horaire de temps d'écran a été mis à jour.",
        "de" => "Dein Bildschirmzeitplan wurde aktualisiert.",
        "pt" => "O horário de ecrã foi atualizado.",
        "pl" => "Harmonogram czasu ekranu został zaktualizowany.",
        _    => "Your allowed screen time schedule has been changed.",
    }
}

pub fn notif_added_body(lang: &str, delta: i32, remaining: i32, reason: Option<&str>) -> String {
    let mins = plural_mins(lang, remaining);
    let base = match lang {
        "es" => format!("+{delta} min. Quedan {remaining} {mins}."),
        "fr" => format!("+{delta} min. Il reste {remaining} {mins}."),
        "de" => format!("+{delta} min. Noch {remaining} {mins}."),
        "pt" => format!("+{delta} min. Restam {remaining} {mins}."),
        "pl" => format!("+{delta} min. Pozostało {remaining} {mins}."),
        _    => format!("+{delta} min added. {remaining} {mins} remaining."),
    };
    match reason {
        Some(r) if !r.is_empty() => format!("{base} ({r})"),
        _ => base,
    }
}

pub fn notif_reduced_body(lang: &str, removed: i32, remaining: i32, reason: Option<&str>) -> String {
    let mins = plural_mins(lang, remaining);
    let base = match lang {
        "es" => format!("−{removed} min. Quedan {remaining} {mins}."),
        "fr" => format!("−{removed} min. Il reste {remaining} {mins}."),
        "de" => format!("−{removed} min. Noch {remaining} {mins}."),
        "pt" => format!("−{removed} min. Restam {remaining} {mins}."),
        "pl" => format!("−{removed} min. Pozostało {remaining} {mins}."),
        _    => format!("−{removed} min removed. {remaining} {mins} remaining."),
    };
    match reason {
        Some(r) if !r.is_empty() => format!("{base} ({r})"),
        _ => base,
    }
}

#[allow(dead_code)]
pub fn notif_message_title(lang: &str) -> &'static str {
    match lang {
        "es" => "Mensaje del administrador",
        "fr" => "Message de l'administrateur",
        "de" => "Nachricht vom Administrator",
        "pt" => "Mensagem do administrador",
        "pl" => "Wiadomość od administratora",
        _    => "Message from administrator",
    }
}

fn plural_mins(lang: &str, n: i32) -> &'static str {
    match lang {
        "pl" => {
            if n == 1 { "minuta" }
            else if (2..=4).contains(&(n.abs() % 10)) && !(11..=14).contains(&(n.abs() % 100)) { "minuty" }
            else { "minut" }
        }
        "es" | "pt" => if n == 1 { "minuto" } else { "minutos" },
        "fr" => if n <= 1 { "minute" } else { "minutes" },
        "de" => if n == 1 { "Minute" } else { "Minuten" },
        _ => if n == 1 { "minute" } else { "minutes" },
    }
}
