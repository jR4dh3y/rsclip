#[derive(Clone, Debug)]
pub struct ColorInfo {
    pub normalized_hex: String,
    pub original_format: String,
    pub rgb: (u8, u8, u8),
}

pub fn parse_color(text: &str) -> Option<ColorInfo> {
    parse_hex(text)
        .or_else(|| parse_rgb(text))
        .or_else(|| parse_named(text))
}

fn parse_hex(text: &str) -> Option<ColorInfo> {
    let raw = text.trim();
    let raw = raw
        .strip_prefix('#')
        .or_else(|| raw.strip_prefix("0x"))
        .or_else(|| raw.strip_prefix("0X"))
        .unwrap_or(raw);
    if !matches!(raw.len(), 3 | 6 | 8) || !raw.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }

    let hex = match raw.len() {
        3 => raw.chars().flat_map(|c| [c, c]).collect::<String>(),
        6 => raw.to_string(),
        8 => raw[0..6].to_string(),
        _ => return None,
    };
    let red = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let green = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let blue = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(ColorInfo {
        normalized_hex: format!("#{hex}").to_ascii_lowercase(),
        original_format: "hex".to_string(),
        rgb: (red, green, blue),
    })
}

fn parse_rgb(text: &str) -> Option<ColorInfo> {
    let text = text.trim();
    let (name, body) = text.split_once('(')?;
    if !matches!(name.trim().to_ascii_lowercase().as_str(), "rgb" | "rgba") {
        return None;
    }
    let body = body.strip_suffix(')')?;
    let mut parts = body.split(',').map(str::trim);
    let red = parse_channel(parts.next()?)?;
    let green = parse_channel(parts.next()?)?;
    let blue = parse_channel(parts.next()?)?;
    if let Some(alpha) = parts.next() {
        parse_alpha(alpha)?;
    }
    if parts.next().is_some() {
        return None;
    }

    Some(ColorInfo {
        normalized_hex: format!("#{red:02x}{green:02x}{blue:02x}"),
        original_format: "rgb".to_string(),
        rgb: (red, green, blue),
    })
}

fn parse_channel(value: &str) -> Option<u8> {
    if value.is_empty() || value.len() > 3 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let parsed = value.parse::<u16>().ok()?;
    u8::try_from(parsed).ok()
}

fn parse_alpha(value: &str) -> Option<()> {
    if value.is_empty() || value.starts_with('+') || value.starts_with('-') {
        return None;
    }
    let alpha = value.parse::<f32>().ok()?;
    (alpha.is_finite() && (0.0..=1.0).contains(&alpha)).then_some(())
}

fn parse_named(text: &str) -> Option<ColorInfo> {
    let (name, rgb) = match text.trim().to_ascii_lowercase().as_str() {
        "black" => ("black", (0, 0, 0)),
        "white" => ("white", (255, 255, 255)),
        "red" => ("red", (255, 0, 0)),
        "green" => ("green", (0, 128, 0)),
        "blue" => ("blue", (0, 0, 255)),
        "transparent" => ("transparent", (0, 0, 0)),
        "rebeccapurple" => ("rebeccapurple", (102, 51, 153)),
        _ => return None,
    };
    Some(ColorInfo {
        normalized_hex: format!("#{:02x}{:02x}{:02x}", rgb.0, rgb.1, rgb.2),
        original_format: name.to_string(),
        rgb,
    })
}

pub fn rgb_text((red, green, blue): (u8, u8, u8)) -> String {
    format!("rgb({red}, {green}, {blue})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hex_shorthand() {
        let color = parse_color("#f0a").unwrap();
        assert_eq!(color.normalized_hex, "#ff00aa");
        assert_eq!(color.rgb, (255, 0, 170));
    }

    #[test]
    fn parses_rgb() {
        let color = parse_color("rgb(195, 251, 91)").unwrap();
        assert_eq!(color.normalized_hex, "#c3fb5b");
    }

    #[test]
    fn parses_rgba_as_rgb() {
        let color = parse_color("rgba(30, 30, 32, 1.0)").unwrap();
        assert_eq!(color.normalized_hex, "#1e1e20");
    }
}
