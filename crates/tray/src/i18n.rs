pub fn tooltip_remaining(lang: &str, time: &str) -> String {
    match lang {
        "pl" => format!("{time} pozostało dziś"),
        "de" => format!("Noch {time} heute"),
        "es" => format!("{time} restantes hoy"),
        "fr" => format!("{time} restantes aujourd'hui"),
        "pt" => format!("{time} restantes hoje"),
        _    => format!("{time} remaining today"),
    }
}

pub fn tooltip_warning(lang: &str, time: &str) -> String {
    match lang {
        "pl" => format!("Uwaga — {time} pozostało"),
        "de" => format!("Warnung — noch {time}"),
        "es" => format!("Aviso — {time} restantes"),
        "fr" => format!("Attention — {time} restantes"),
        "pt" => format!("Aviso — {time} restantes"),
        _    => format!("Warning — {time} remaining"),
    }
}

pub fn tooltip_locked(lang: &str) -> &'static str {
    match lang {
        "pl" => "Limit czasu ekranu osiągnięty",
        "de" => "Bildschirmzeit-Limit erreicht",
        "es" => "Límite de tiempo de pantalla alcanzado",
        "fr" => "Limite de temps d'écran atteinte",
        "pt" => "Limite de tempo de tela atingido",
        _    => "Screen time limit reached",
    }
}

pub fn tooltip_unlimited(lang: &str) -> &'static str {
    match lang {
        "pl" => "Brak limitu czasu dziś",
        "de" => "Kein Zeitlimit heute",
        "es" => "Sin límite de tiempo hoy",
        "fr" => "Pas de limite de temps aujourd'hui",
        "pt" => "Sem limite de tempo hoje",
        _    => "No time limit today",
    }
}

pub fn tooltip_unavailable(lang: &str) -> &'static str {
    match lang {
        "pl" => "Status niedostępny",
        "de" => "Status nicht verfügbar",
        "es" => "Estado no disponible",
        "fr" => "Statut indisponible",
        "pt" => "Status indisponível",
        _    => "Status unavailable",
    }
}
