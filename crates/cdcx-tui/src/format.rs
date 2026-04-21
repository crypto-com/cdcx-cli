pub fn format_price(price: f64) -> String {
    if price == 0.0 {
        return "\u{2014}".into();
    }
    if price >= 1000.0 {
        let s = format!("{:.2}", price);
        let parts: Vec<&str> = s.split('.').collect();
        let int_part = parts[0];
        let dec_part = parts[1];
        let digits: Vec<char> = int_part.chars().collect();
        let mut result = String::new();
        for (i, &c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                result.push(',');
            }
            result.push(c);
        }
        format!("{}.{}", result, dec_part)
    } else if price >= 1.0 {
        format!("{:.4}", price)
    } else if price >= 0.01 {
        format!("{:.6}", price)
    } else {
        format!("{:.8}", price)
    }
}

pub fn format_compact(value: f64) -> String {
    if value == 0.0 {
        return "\u{2014}".into();
    }
    if value >= 1_000_000_000.0 {
        format!("{:.1}B", value / 1_000_000_000.0)
    } else if value >= 1_000_000.0 {
        format!("{:.1}M", value / 1_000_000.0)
    } else if value >= 1_000.0 {
        format!("{:.1}K", value / 1_000.0)
    } else {
        format!("{:.0}", value)
    }
}
