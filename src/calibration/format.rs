pub fn table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|header| header.len()).collect();
    for row in rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.chars().count());
        }
    }

    let mut out = String::new();
    write_row(&mut out, headers.iter().copied(), &widths);
    let divider: Vec<String> = widths.iter().map(|width| "-".repeat(*width)).collect();
    write_row(&mut out, divider.iter().map(String::as_str), &widths);
    for row in rows {
        write_row(&mut out, row.iter().map(String::as_str), &widths);
    }
    out
}

pub fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut truncated = value.chars().take(keep).collect::<String>();
    truncated.push('…');
    truncated
}

fn write_row<'a>(out: &mut String, cells: impl Iterator<Item = &'a str>, widths: &[usize]) {
    for (idx, cell) in cells.enumerate() {
        if idx > 0 {
            out.push_str("  ");
        }
        out.push_str(cell);
        let padding = widths[idx].saturating_sub(cell.chars().count());
        out.push_str(&" ".repeat(padding));
    }
    out.push('\n');
}
