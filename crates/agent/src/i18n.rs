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

/// Name of an app group: predefined groups in the child's language unless the
/// parent renamed them.
pub fn group_name(builtin: Option<&str>, name: &str, lang: &str) -> String {
    let localized = match (builtin, name) {
        (Some("games"), "Games") => match lang {
            "es" => "Juegos",
            "fr" => "Jeux",
            "de" => "Spiele",
            "pt" => "Jogos",
            "pl" => "Gry",
            _ => "Games",
        },
        (Some("browsers"), "Web browsers") => match lang {
            "es" => "Navegadores web",
            "fr" => "Navigateurs web",
            "de" => "Webbrowser",
            "pt" => "Navegadores web",
            "pl" => "Przeglądarki",
            _ => "Web browsers",
        },
        _ => return name.to_string(),
    };
    localized.to_string()
}

/// "Games: 5 minutes left today"
pub fn notif_group_warning_body(lang: &str, remaining: i32) -> String {
    let mins = plural_mins(lang, remaining);
    match lang {
        "es" => format!("Quedan {remaining} {mins} por hoy."),
        "fr" => format!("Il reste {remaining} {mins} aujourd'hui."),
        "de" => format!("Heute noch {remaining} {mins}."),
        "pt" => format!("Restam {remaining} {mins} por hoje."),
        "pl" => format!("Na dziś zostało {remaining} {mins}."),
        _ => format!("{remaining} {mins} left today."),
    }
}

/// Title for app-group notifications: "Games: time is up"
pub fn notif_group_time_up_title(lang: &str, group: &str) -> String {
    match lang {
        "es" => format!("{group}: se acabó el tiempo"),
        "fr" => format!("{group} : temps écoulé"),
        "de" => format!("{group}: Zeit ist um"),
        "pt" => format!("{group}: o tempo acabou"),
        "pl" => format!("{group}: czas minął"),
        _ => format!("{group}: time is up"),
    }
}

/// "Dota 2 will close in 10 seconds."
pub fn notif_app_closing_body(lang: &str, app: &str, secs: u64) -> String {
    match lang {
        "es" => format!("{app} se cerrará en {secs} segundos."),
        "fr" => format!("{app} va se fermer dans {secs} secondes."),
        "de" => format!("{app} wird in {secs} Sekunden geschlossen."),
        "pt" => format!("{app} vai fechar dentro de {secs} segundos."),
        "pl" => format!("{app} zostanie zamknięta za {secs} sekund."),
        _ => format!("{app} will close in {secs} seconds."),
    }
}

/// Title when a group is blocked for the day: "Games: not allowed today"
pub fn notif_group_blocked_title(lang: &str, group: &str) -> String {
    match lang {
        "es" => format!("{group}: no permitido hoy"),
        "fr" => format!("{group} : pas autorisé aujourd'hui"),
        "de" => format!("{group}: heute nicht erlaubt"),
        "pt" => format!("{group}: não permitido hoje"),
        "pl" => format!("{group}: dziś niedozwolone"),
        _ => format!("{group}: not allowed today"),
    }
}

/// "Dota 2 was closed."
pub fn notif_app_closed_body(lang: &str, app: &str) -> String {
    match lang {
        "es" => format!("{app} se ha cerrado."),
        "fr" => format!("{app} a été fermée."),
        "de" => format!("{app} wurde geschlossen."),
        "pt" => format!("{app} foi fechada."),
        "pl" => format!("{app} została zamknięta."),
        _ => format!("{app} was closed."),
    }
}

/// "15 extra minutes today." (or fewer, when negative)
pub fn notif_group_extra_body(lang: &str, delta: i32) -> String {
    let n = delta.abs();
    let mins = plural_mins(lang, n);
    match (lang, delta > 0) {
        ("es", true) => format!("{n} {mins} más por hoy."),
        ("es", false) => format!("{n} {mins} menos por hoy."),
        ("fr", true) => format!("{n} {mins} de plus aujourd'hui."),
        ("fr", false) => format!("{n} {mins} de moins aujourd'hui."),
        ("de", true) => format!("Heute {n} {mins} mehr."),
        ("de", false) => format!("Heute {n} {mins} weniger."),
        ("pt", true) => format!("Mais {n} {mins} por hoje."),
        ("pt", false) => format!("Menos {n} {mins} por hoje."),
        ("pl", true) => format!("Dodatkowe {n} {mins} na dziś."),
        ("pl", false) => format!("{n} {mins} mniej na dziś."),
        (_, true) => format!("{n} extra {mins} today."),
        (_, false) => format!("{n} {mins} less today."),
    }
}

/// Title when an app needs a parent's approval: "Ask a parent"
pub fn notif_app_needs_approval_title(lang: &str) -> &'static str {
    match lang {
        "es" => "Hace falta permiso",
        "fr" => "Autorisation nécessaire",
        "de" => "Erlaubnis nötig",
        "pt" => "É preciso autorização",
        "pl" => "Potrzebna zgoda",
        _ => "Permission needed",
    }
}

/// "Your parent was asked whether you may use Dota 2."
pub fn notif_app_needs_approval_body(lang: &str, app: &str) -> String {
    match lang {
        "es" => format!("Se ha pedido a tus padres que permitan {app}."),
        "fr" => format!("Une demande a été envoyée à tes parents pour {app}."),
        "de" => format!("Deine Eltern wurden gefragt, ob du {app} benutzen darfst."),
        "pt" => format!("Foi pedido aos teus pais que permitam {app}."),
        "pl" => format!("Wysłano rodzicom prośbę o zgodę na {app}."),
        _ => format!("Your parents were asked to allow {app}."),
    }
}

/// Title when a parent said no / the app isn't allowed: "Not allowed"
pub fn notif_app_denied_title(lang: &str) -> &'static str {
    match lang {
        "es" => "No permitido",
        "fr" => "Non autorisé",
        "de" => "Nicht erlaubt",
        "pt" => "Não permitido",
        "pl" => "Niedozwolone",
        _ => "Not allowed",
    }
}

/// "You may use Dota 2 now."
pub fn notif_app_approved_body(lang: &str, app: &str) -> String {
    match lang {
        "es" => format!("Ya puedes usar {app}."),
        "fr" => format!("Tu peux maintenant utiliser {app}."),
        "de" => format!("Du darfst {app} jetzt benutzen."),
        "pt" => format!("Já podes usar {app}."),
        "pl" => format!("Możesz już używać {app}."),
        _ => format!("You may use {app} now."),
    }
}

/// Title when a parent allowed an app: "Allowed"
pub fn notif_app_approved_title(lang: &str) -> &'static str {
    match lang {
        "es" => "Permitido",
        "fr" => "Autorisé",
        "de" => "Erlaubt",
        "pt" => "Permitido",
        "pl" => "Dozwolone",
        _ => "Allowed",
    }
}
