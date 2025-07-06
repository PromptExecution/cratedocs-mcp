use regex::Regex;

/// Remove LICENSE and VERSION(S) sections by skipping lines between those headings and the next heading or EOF.
/// Also removes <detail> tags.
pub fn apply_tldr(input: &str) -> String {
    let mut output = Vec::new();
    let mut skip = false;

    // Match any heading (with or without space) for LICENSE or VERSION(S)
    let tldr_section_re = Regex::new(r"(?i)^\s*#+\s*(license|version(s)?)\b").unwrap();
    // Match any heading (for ending the skip)
    let heading_re = Regex::new(r"^\s*#+").unwrap();
    // Match <detail> tags including start, end, and inline attributes
    let detail_tag_re = Regex::new(r"<[/]?detail.*?>").unwrap();

    for line in input.lines() {
        // Start skipping if we hit a LICENSE or VERSION(S) heading
        if !skip && tldr_section_re.is_match(line) {
            skip = true;
            continue; // skip the heading line itself
        }
        // Stop skipping at the next heading (but do not skip the heading itself)
        if skip && heading_re.is_match(line) {
            skip = false;
        }
        if !skip {
            // Remove <detail> tags from the line
            let cleaned_line = detail_tag_re.replace_all(line, "").to_string();
            output.push(cleaned_line);
        }
    }
    output.join("\n")
}