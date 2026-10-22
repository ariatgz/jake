//! Reads a resume written with the "Jake's Resume" LaTeX macros and rewrites it
//! with bullets, projects and skills reordered or removed. Only those spans are
//! touched; everything else in the file is copied through unchanged.
//!
//! Optional directives go in a LaTeX comment on the line above (or at the end
//! of) a bullet or project heading:
//!   `% @keep`             never drop this item
//!   `% @tags: FPGA, RTL`  extra keywords to match against postings

use std::ops::Range;

const ENTRY_COMMANDS: &[&str] = &["\\resumeSubheading", "\\resumeProjectHeading"];
const BULLET_COMMANDS: &[&str] = &["\\resumeItem", "\\resumeSubItem"];
const ENTRY_LIST_END: &str = "\\resumeSubHeadingListEnd";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    Education,
    Experience,
    Projects,
    Skills,
    Other,
}

impl SectionKind {
    fn from_title(title: &str) -> Self {
        let t = title.to_lowercase();
        if t.contains("project") {
            SectionKind::Projects
        } else if t.contains("skill") || t.contains("technolog") || t.contains("tools") {
            SectionKind::Skills
        } else if t.contains("education") || t.contains("academic") {
            SectionKind::Education
        } else if t.contains("experience") || t.contains("employment") || t.contains("work") {
            SectionKind::Experience
        } else {
            SectionKind::Other
        }
    }
}

#[derive(Debug, Clone)]
pub struct Bullet {
    span: Range<usize>,
    pub text: String,
    pub pinned: bool,
    pub tags: Vec<String>,
    pub list: usize,
}

#[derive(Debug, Clone)]
pub struct Entry {
    span: Range<usize>,
    /// All heading fields as text, for keyword matching.
    pub heading: String,
    /// Short name for reports, e.g. "Firmware Intern @ Acme".
    pub label: String,
    /// First heading field: job title, school or project name.
    pub name: String,
    /// Where it happened: the company for experience, otherwise `name`.
    pub org: String,
    pub pinned: bool,
    pub tags: Vec<String>,
    pub lists: Vec<usize>,
    pub section: usize,
}

#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub kind: SectionKind,
    pub entries: Vec<usize>,
    /// Bullet lists in the section that aren't under an entry heading.
    pub loose_lists: Vec<usize>,
}

#[derive(Debug, Clone)]
pub struct SkillLine {
    /// Everything after the colon, up to the end of the group or line.
    span: Range<usize>,
    pub category: String,
    pub items: Vec<String>,
    leading: String,
    trailing: String,
}

#[derive(Debug, Clone)]
pub struct Document {
    src: String,
    pub sections: Vec<Section>,
    pub entries: Vec<Entry>,
    pub bullets: Vec<Bullet>,
    /// Bullet ids per list, in document order.
    pub lists: Vec<Vec<usize>>,
    pub skill_lines: Vec<SkillLine>,
    body: Range<usize>,
}

/// What to keep and in which order.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Kept entry ids for each section, in output order.
    pub entry_order: Vec<Vec<usize>>,
    /// Kept bullet ids for each list, in output order.
    pub list_order: Vec<Vec<usize>>,
    /// Items for each skill line, in output order.
    pub skill_order: Vec<Vec<String>>,
}

/// Copy of `src` with comments blanked out, so searches never match commented
/// code. Byte offsets are identical to `src`.
fn mask_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut in_comment = false;
    let mut backslashes = 0usize;
    for c in src.chars() {
        if c == '\n' {
            in_comment = false;
            backslashes = 0;
            out.push('\n');
            continue;
        }
        if in_comment {
            for _ in 0..c.len_utf8() {
                out.push(' ');
            }
            continue;
        }
        if c == '%' && backslashes % 2 == 0 {
            in_comment = true;
            out.push(' ');
            continue;
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
        out.push(c);
    }
    out
}

fn line_start(s: &str, pos: usize) -> usize {
    s[..pos].rfind('\n').map_or(0, |i| i + 1)
}

/// Index just past the newline ending the line that contains `pos`.
fn line_end_inclusive(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    s[pos..].find('\n').map_or(s.len(), |i| pos + i + 1)
}

/// Occurrences of a control word that aren't a prefix of a longer one
/// (`\resumeItem` must not match `\resumeItemListStart`).
fn find_command(masked: &str, name: &str, range: Range<usize>) -> Vec<usize> {
    let hay = &masked[range.clone()];
    hay.match_indices(name)
        .map(|(i, _)| range.start + i)
        .filter(|&p| !masked[p + name.len()..].starts_with(|c: char| c.is_ascii_alphabetic() || c == '@'))
        .collect()
}

/// Given the index of `{`, returns the index just past the matching `}`.
fn group_end(masked: &str, open: usize) -> Option<usize> {
    let bytes = masked.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 2;
                continue;
            }
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Skips spaces/tabs and at most one newline; a blank line ends a command's arguments.
fn skip_inline_space(masked: &str, mut i: usize) -> usize {
    let bytes = masked.as_bytes();
    let mut newlines = 0;
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\r' => i += 1,
            b'\n' if newlines == 0 => {
                newlines += 1;
                i += 1;
            }
            _ => break,
        }
    }
    i
}

/// Consecutive `{...}` groups starting at `from`; returns their ranges.
fn groups(masked: &str, from: usize, max: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut i = from;
    while out.len() < max {
        let j = skip_inline_space(masked, i);
        match group_end(masked, j) {
            Some(end) => {
                out.push(j..end);
                i = end;
            }
            None => break,
        }
    }
    out
}

fn is_directive_line(src: &str, line: Range<usize>) -> bool {
    let t = src[line].trim();
    t.starts_with('%') && t.trim_start_matches('%').trim_start().starts_with('@')
}

/// Moves `start` (a line start) up over directive comment lines directly above it.
fn extend_over_directives(src: &str, mut start: usize) -> usize {
    while start > 0 {
        let prev_start = line_start(src, start - 1);
        if is_directive_line(src, prev_start..start) {
            start = prev_start;
        } else {
            break;
        }
    }
    start
}

/// Reads `@keep` and `@tags:` from the comments inside `span`.
fn directives(src: &str, span: Range<usize>) -> (bool, Vec<String>) {
    let mut pinned = false;
    let mut tags = Vec::new();
    for line in src[span].lines() {
        let Some(idx) = comment_index(line) else { continue };
        let comment = line[idx..].trim_start_matches('%');
        for part in comment.split('@').skip(1) {
            let part = part.trim();
            let lower = part.to_lowercase();
            if lower.starts_with("keep") {
                pinned = true;
            } else if lower.starts_with("tags") {
                let rest = part[4..].trim_start_matches([':', ' ']);
                tags.extend(rest.split(',').map(|t| t.trim().to_string()).filter(|t| !t.is_empty()));
            }
        }
    }
    (pinned, tags)
}

fn comment_index(line: &str) -> Option<usize> {
    let mut backslashes = 0;
    for (i, c) in line.char_indices() {
        if c == '%' && backslashes % 2 == 0 {
            return Some(i);
        }
        backslashes = if c == '\\' { backslashes + 1 } else { 0 };
    }
    None
}

/// LaTeX source -> readable text for keyword matching ("\textbf{C\texttt{++}}" -> "C++").
pub fn plain_text(latex: &str) -> String {
    let masked = mask_comments(latex);
    let mut out = String::with_capacity(masked.len());
    let mut chars = masked.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.peek().copied() {
                Some(n) if n.is_ascii_alphabetic() => {
                    while chars.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
                        chars.next();
                    }
                    if chars.peek() == Some(&'*') {
                        chars.next();
                    }
                }
                Some('\\') => {
                    chars.next();
                    out.push(' ');
                }
                Some(n) => {
                    chars.next();
                    out.push(if n == ',' || n == ' ' { ' ' } else { n });
                }
                None => {}
            },
            '{' | '}' | '$' => {}
            '~' => out.push(' '),
            _ => out.push(c),
        }
    }
    out.replace("--", "-").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Splits on commas that aren't inside braces or parentheses.
fn split_items(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut prev_backslash = false;
    for c in s.chars() {
        match c {
            '{' | '(' | '[' if !prev_backslash => depth += 1,
            '}' | ')' | ']' if !prev_backslash => depth -= 1,
            ',' if depth == 0 => {
                out.push(current.trim().to_string());
                current.clear();
                prev_backslash = false;
                continue;
            }
            _ => {}
        }
        prev_backslash = c == '\\' && !prev_backslash;
        current.push(c);
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_string());
    }
    out.retain(|s| !s.is_empty());
    out
}

impl Document {
    pub fn parse(src: &str) -> Result<Document, String> {
        let masked = mask_comments(src);
        let body_start = masked
            .find("\\begin{document}")
            .map(|i| i + "\\begin{document}".len())
            .ok_or("No \\begin{document} found. Upload the whole .tex file.")?;
        let body_end = masked.find("\\end{document}").unwrap_or(masked.len());
        if body_end < body_start {
            return Err("\\end{document} appears before \\begin{document}.".into());
        }

        // Sections --------------------------------------------------------
        let section_starts: Vec<usize> = find_command(&masked, "\\section", body_start..body_end);
        let mut sections = Vec::new();
        let mut section_spans: Vec<Range<usize>> = Vec::new();
        for (i, &pos) in section_starts.iter().enumerate() {
            let after = pos + "\\section".len();
            let after = if masked[after..].starts_with('*') { after + 1 } else { after };
            let title = groups(&masked, after, 1).first().map(|g| plain_text(&src[g.start + 1..g.end - 1])).unwrap_or_default();
            let end = section_starts.get(i + 1).map_or(body_end, |&n| line_start(src, n));
            sections.push(Section { kind: SectionKind::from_title(&title), title, entries: Vec::new(), loose_lists: Vec::new() });
            section_spans.push(line_start(src, pos)..end);
        }
        let section_of = |pos: usize| section_spans.iter().position(|s| s.contains(&pos));

        // Entries ---------------------------------------------------------
        let mut entries: Vec<Entry> = Vec::new();
        for (s_idx, s_span) in section_spans.iter().enumerate() {
            let mut heads: Vec<usize> = ENTRY_COMMANDS.iter().flat_map(|c| find_command(&masked, c, s_span.clone())).collect();
            heads.sort_unstable();
            let starts: Vec<usize> = heads.iter().map(|&h| extend_over_directives(src, line_start(src, h))).collect();
            for (k, &head) in heads.iter().enumerate() {
                let mut end = starts.get(k + 1).copied().unwrap_or(s_span.end);
                if let Some(list_end) = masked[head..end].find(ENTRY_LIST_END) {
                    end = line_start(src, head + list_end);
                }
                let span = starts[k]..end;
                let name_len = ENTRY_COMMANDS.iter().find(|c| masked[head..].starts_with(*c)).map_or(0, |c| c.len());
                let args = groups(&masked, head + name_len, 4);
                let heading_end = args.last().map_or(head + name_len, |g| g.end);
                let parts: Vec<String> = args.iter().map(|g| plain_text(&src[g.start + 1..g.end - 1])).collect();
                let heading = parts.join(" | ");
                // "Title @ Company" for \resumeSubheading{title}{date}{company}{place};
                // the name before "|" for project headings.
                let first = parts.first().map(|p| p.split('|').next().unwrap_or(p).trim().to_string()).unwrap_or_default();
                let company = parts.get(2).map(|c| c.trim().to_string()).filter(|c| parts.len() == 4 && !c.is_empty());
                let label = match &company {
                    Some(company) => format!("{first} @ {company}"),
                    None => first.clone(),
                };
                let org = company.unwrap_or_else(|| first.clone());
                let directive_span = span.start..line_end_inclusive(src, heading_end);
                let (pinned, tags) = directives(src, directive_span);
                sections[s_idx].entries.push(entries.len());
                entries.push(Entry { span, heading, label, name: first, org, pinned, tags, lists: Vec::new(), section: s_idx });
            }
        }

        // Bullets and lists ----------------------------------------------
        let mut bullet_positions: Vec<(usize, usize)> = BULLET_COMMANDS
            .iter()
            .flat_map(|c| find_command(&masked, c, body_start..body_end).into_iter().map(move |p| (p, c.len())))
            .collect();
        bullet_positions.sort_unstable();

        let mut bullets: Vec<Bullet> = Vec::new();
        for (pos, name_len) in bullet_positions {
            let args = groups(&masked, pos + name_len, 2);
            let Some(last) = args.last() else { continue };
            let close = last.end;
            let ls = line_start(src, pos);
            let start = if masked[ls..pos].trim().is_empty() { extend_over_directives(src, ls) } else { pos };
            let le = line_end_inclusive(src, close);
            let end = if masked[close..le].trim().is_empty() { le } else { close };
            let text = args.iter().map(|g| plain_text(&src[g.start + 1..g.end - 1])).collect::<Vec<_>>().join(" ");
            let (pinned, tags) = directives(src, start..end);
            bullets.push(Bullet { span: start..end, text, pinned, tags, list: 0 });
        }

        let mut lists: Vec<Vec<usize>> = Vec::new();
        for i in 0..bullets.len() {
            let joins_previous = i > 0 && {
                let gap = &masked[bullets[i - 1].span.end..bullets[i].span.start];
                gap.trim().is_empty()
            };
            if !joins_previous {
                lists.push(Vec::new());
            }
            let list_id = lists.len() - 1;
            lists[list_id].push(i);
            bullets[i].list = list_id;
        }

        for (list_id, ids) in lists.iter().enumerate() {
            let first = bullets[ids[0]].span.start;
            let last = bullets[*ids.last().unwrap()].span.end;
            if let Some(e) = entries.iter_mut().find(|e| e.span.start <= first && last <= e.span.end) {
                e.lists.push(list_id);
            } else if let Some(s) = section_of(first) {
                sections[s].loose_lists.push(list_id);
            }
        }

        // Skill lines -----------------------------------------------------
        let mut skill_lines = Vec::new();
        for (s_idx, s_span) in section_spans.iter().enumerate() {
            if sections[s_idx].kind != SectionKind::Skills {
                continue;
            }
            for pos in find_command(&masked, "\\textbf", s_span.clone()) {
                if entries.iter().any(|e| e.span.contains(&pos)) {
                    continue;
                }
                let Some(cat) = groups(&masked, pos + "\\textbf".len(), 1).pop() else { continue };
                let category_raw = plain_text(&src[cat.start + 1..cat.end - 1]);
                let colon_in_category = category_raw.trim_end().ends_with(':');
                let next = skip_inline_space(&masked, cat.end);
                let (content, braced) = match group_end(&masked, next) {
                    Some(end) if !colon_in_category => (next + 1..end - 1, true),
                    _ => {
                        let line_end = masked[cat.end..].find('\n').map_or(s_span.end, |i| cat.end + i);
                        let stop = masked[cat.end..line_end].find("\\\\").map_or(line_end, |i| cat.end + i);
                        (cat.end..stop.min(s_span.end), false)
                    }
                };
                let text = &masked[content.clone()];
                let items_start = if colon_in_category {
                    content.start
                } else {
                    match text.trim_start().strip_prefix(':') {
                        Some(_) => content.start + text.find(':').unwrap() + 1,
                        None => continue,
                    }
                };
                // Line-based items must not run into a following \textbf.
                let items_end = if braced {
                    content.end
                } else {
                    masked[items_start..content.end].find("\\textbf").map_or(content.end, |i| items_start + i)
                };
                let raw = &src[items_start..items_end];
                let trimmed = raw.trim();
                let items = split_items(trimmed);
                if items.is_empty() {
                    continue;
                }
                let leading = &raw[..raw.len() - raw.trim_start().len()];
                let trailing = &raw[raw.trim_end().len()..];
                skill_lines.push(SkillLine {
                    span: items_start..items_end,
                    category: category_raw.trim_end_matches(':').trim().to_string(),
                    items,
                    leading: leading.to_string(),
                    trailing: trailing.to_string(),
                });
            }
        }

        Ok(Document { src: src.to_string(), sections, entries, bullets, lists, skill_lines, body: body_start..body_end })
    }

    /// The untouched plan: same content, same order.
    pub fn identity_plan(&self) -> Plan {
        Plan {
            entry_order: self.sections.iter().map(|s| s.entries.clone()).collect(),
            list_order: self.lists.clone(),
            skill_order: self.skill_lines.iter().map(|l| l.items.clone()).collect(),
        }
    }

    /// Readable text of the whole resume body.
    pub fn body_text(&self) -> String {
        plain_text(&self.src[self.body.clone()])
    }

    /// The name in the header: the first bold text before any section.
    pub fn person_name(&self) -> Option<String> {
        let masked = mask_comments(&self.src);
        let header_end = find_command(&masked, "\\section", self.body.clone()).first().copied().unwrap_or(self.body.end);
        let pos = find_command(&masked, "\\textbf", self.body.start..header_end).into_iter().next()?;
        let group = groups(&masked, pos + "\\textbf".len(), 1).pop()?;
        let name = plain_text(&self.src[group.start + 1..group.end - 1]);
        let words = name.split_whitespace().count();
        (1..=5).contains(&words).then_some(name)
    }

    /// The first school listed under Education.
    pub fn school(&self) -> Option<String> {
        self.sections
            .iter()
            .filter(|s| s.kind == SectionKind::Education)
            .flat_map(|s| s.entries.iter())
            .map(|&e| self.entries[e].name.clone())
            .find(|n| !n.is_empty())
    }

    /// Every skill item listed in the skills section(s), split on `/` as well
    /// ("C/C++" -> "C", "C++").
    pub fn listed_skills(&self) -> Vec<String> {
        let mut out = Vec::new();
        for line in &self.skill_lines {
            for item in &line.items {
                let text = plain_text(item);
                let base = text.split('(').next().unwrap_or(&text).trim().to_string();
                out.push(base.clone());
                if base.contains('/') {
                    out.extend(base.split('/').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()));
                }
                if let (Some(open), Some(close)) = (text.find('('), text.rfind(')')) {
                    out.extend(text[open + 1..close].split(['/', ',']).map(|p| p.trim().to_string()).filter(|p| !p.is_empty()));
                }
            }
        }
        for bullet in &self.bullets {
            out.extend(bullet.tags.iter().cloned());
        }
        for entry in &self.entries {
            out.extend(entry.tags.iter().cloned());
        }
        out.sort();
        out.dedup();
        out
    }

    pub fn render(&self, plan: &Plan) -> String {
        let mut edits: Vec<(Range<usize>, String)> = Vec::new();

        for (s_idx, section) in self.sections.iter().enumerate() {
            let order = &plan.entry_order[s_idx];
            for (slot, &original) in section.entries.iter().enumerate() {
                let replacement = order.get(slot).map(|&e| self.render_entry(e, plan)).unwrap_or_default();
                edits.push((self.entries[original].span.clone(), replacement));
            }
            for &list in &section.loose_lists {
                edits.extend(self.list_edits(list, plan));
            }
        }
        for (i, line) in self.skill_lines.iter().enumerate() {
            let items = &plan.skill_order[i];
            edits.push((line.span.clone(), format!("{}{}{}", line.leading, items.join(", "), line.trailing)));
        }

        apply_edits(&self.src, 0..self.src.len(), edits)
    }

    fn render_entry(&self, entry: usize, plan: &Plan) -> String {
        let e = &self.entries[entry];
        let edits = e.lists.iter().flat_map(|&l| self.list_edits(l, plan)).collect();
        apply_edits(&self.src, e.span.clone(), edits)
    }

    fn list_edits(&self, list: usize, plan: &Plan) -> Vec<(Range<usize>, String)> {
        self.lists[list]
            .iter()
            .enumerate()
            .map(|(slot, &original)| {
                let slot_span = self.bullets[original].span.clone();
                let replacement = match plan.list_order[list].get(slot) {
                    Some(&b) => {
                        // A bullet moved into a slot that ended a line must end one too.
                        let mut text = self.src[self.bullets[b].span.clone()].to_string();
                        if self.src[slot_span.clone()].ends_with('\n') && !text.ends_with('\n') {
                            text.push('\n');
                        }
                        text
                    }
                    None => String::new(),
                };
                (slot_span, replacement)
            })
            .collect()
    }
}

fn apply_edits(src: &str, range: Range<usize>, mut edits: Vec<(Range<usize>, String)>) -> String {
    edits.retain(|(r, _)| range.start <= r.start && r.end <= range.end);
    edits.sort_by_key(|(r, _)| r.start);
    let mut out = String::with_capacity(range.len());
    let mut cursor = range.start;
    for (r, text) in edits {
        if r.start < cursor {
            continue; // overlapping edit; keep the first
        }
        out.push_str(&src[cursor..r.start]);
        out.push_str(&text);
        cursor = r.end;
    }
    out.push_str(&src[cursor..range.end]);
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub const SAMPLE: &str = r#"\documentclass{article}
\newcommand{\resumeItem}[1]{\item\small{{#1}}}
\newcommand{\resumeSubheading}[4]{\item #1 #2 #3 #4}
\newcommand{\resumeProjectHeading}[2]{\item #1 #2}
\newcommand{\resumeSubHeadingListStart}{\begin{itemize}}
\newcommand{\resumeSubHeadingListEnd}{\end{itemize}}
\newcommand{\resumeItemListStart}{\begin{itemize}}
\newcommand{\resumeItemListEnd}{\end{itemize}}
\begin{document}
\section{Education}
  \resumeSubHeadingListStart
    \resumeSubheading
      {University of Waterloo}{Waterloo, ON}
      {BASc Computer Engineering}{2023 -- 2028}
  \resumeSubHeadingListEnd
\section{Experience}
  \resumeSubHeadingListStart
    \resumeSubheading
      {Firmware Intern}{May 2025 -- Aug 2025}
      {Acme Robotics}{Toronto, ON}
      \resumeItemListStart
        \resumeItem{Built a React dashboard for factory metrics}
        \resumeItem{Wrote \textbf{C} drivers for I2C and SPI sensors on an STM32, cutting boot time 40\%} % @keep
        % @tags: Python
        \resumeItem{Automated hardware-in-the-loop regression tests}
      \resumeItemListEnd
%    \resumeSubheading{Commented}{Out}{Entry}{Here}
  \resumeSubHeadingListEnd
\section{Projects}
    \resumeSubHeadingListStart
      \resumeProjectHeading
          {\textbf{Portfolio Site} $|$ \emph{React, Node.js}}{2024}
          \resumeItemListStart
            \resumeItem{Personal website with a blog}
          \resumeItemListEnd
      \resumeProjectHeading
          {\textbf{RISC-V Core} $|$ \emph{SystemVerilog, Vivado}}{2025}
          \resumeItemListStart
            \resumeItem{Designed a 5-stage pipelined RISC-V CPU in SystemVerilog on an FPGA}
            \resumeItem{Wrote UVM-style testbenches reaching 95\% coverage}
          \resumeItemListEnd
    \resumeSubHeadingListEnd
\section{Technical Skills}
 \begin{itemize}
    \small{\item{
     \textbf{Languages}{: Python, C/C++, SystemVerilog, JavaScript} \\
     \textbf{Tools}{: Git, Vivado, Docker}
    }}
 \end{itemize}
\end{document}
"#;

    #[test]
    fn parses_structure() {
        let doc = Document::parse(SAMPLE).unwrap();
        let titles: Vec<_> = doc.sections.iter().map(|s| (s.title.as_str(), s.kind)).collect();
        assert_eq!(
            titles,
            vec![
                ("Education", SectionKind::Education),
                ("Experience", SectionKind::Experience),
                ("Projects", SectionKind::Projects),
                ("Technical Skills", SectionKind::Skills)
            ]
        );
        assert_eq!(doc.entries.len(), 4, "commented-out entry is ignored");
        assert_eq!(doc.bullets.len(), 6);
        assert_eq!(doc.lists.len(), 3);
        assert!(doc.bullets[1].pinned);
        assert!(doc.bullets[1].text.contains("C drivers for I2C"));
        assert!(doc.bullets[1].text.contains("40%"));
        assert_eq!(doc.bullets[2].tags, vec!["Python"]);
        assert!(doc.entries[3].heading.contains("SystemVerilog"));
        assert_eq!(doc.entries[1].label, "Firmware Intern @ Acme Robotics");
        assert_eq!(doc.entries[1].org, "Acme Robotics");
        assert_eq!(doc.entries[3].label, "RISC-V Core");
        assert_eq!(doc.school().as_deref(), Some("University of Waterloo"));
        assert_eq!(doc.person_name(), None, "sample has no header");
        assert_eq!(doc.skill_lines.len(), 2);
        assert_eq!(doc.skill_lines[0].category, "Languages");
        assert_eq!(doc.skill_lines[0].items, vec!["Python", "C/C++", "SystemVerilog", "JavaScript"]);
        let listed = doc.listed_skills();
        assert!(listed.contains(&"C++".to_string()) && listed.contains(&"Python".to_string()));
    }

    #[test]
    fn identity_round_trip() {
        let doc = Document::parse(SAMPLE).unwrap();
        assert_eq!(doc.render(&doc.identity_plan()), SAMPLE);
    }

    #[test]
    fn reorders_and_drops() {
        let doc = Document::parse(SAMPLE).unwrap();
        let mut plan = doc.identity_plan();
        plan.list_order[0] = vec![1, 2]; // drop the React bullet, keep pinned first
        plan.entry_order[2] = vec![3]; // keep only the RISC-V project
        plan.skill_order[0] = vec!["SystemVerilog".into(), "C/C++".into(), "Python".into(), "JavaScript".into()];
        let out = doc.render(&plan);

        assert!(!out.contains("React dashboard"));
        assert!(!out.contains("Portfolio Site"));
        assert!(out.contains("\\textbf{Languages}{: SystemVerilog, C/C++, Python, JavaScript} \\\\"));
        let drivers = out.find("C} drivers").unwrap();
        let automated = out.find("Automated hardware").unwrap();
        assert!(drivers < automated);
        assert!(out.contains("% @tags: Python\n        \\resumeItem{Automated"), "directive moves with its bullet");
        assert!(out.contains("\\resumeItemListEnd\n%    \\resumeSubheading{Commented}"), "surrounding text untouched");
        // Re-parsing the output gives a consistent document.
        let again = Document::parse(&out).unwrap();
        assert_eq!(again.bullets.len(), 4);
        assert_eq!(again.entries.len(), 3);
    }

    #[test]
    fn plain_text_strips_markup() {
        assert_eq!(plain_text(r"Cut \textbf{latency} by 40\% using C\texttt{++} \& Rust"), "Cut latency by 40% using C++ & Rust");
        assert_eq!(plain_text(r"\textbf{Gitlytics} $|$ \emph{Python, Flask}"), "Gitlytics | Python, Flask");
    }

    #[test]
    fn colon_inside_bold_category() {
        let src = "\\begin{document}\n\\section{Skills}\n\\textbf{Languages:} Rust, Go, C \\\\\n\\textbf{Tools:} Git, Linux\n\\end{document}\n";
        let doc = Document::parse(src).unwrap();
        assert_eq!(doc.skill_lines.len(), 2);
        assert_eq!(doc.skill_lines[0].items, vec!["Rust", "Go", "C"]);
        assert_eq!(doc.skill_lines[1].items, vec!["Git", "Linux"]);
        let mut plan = doc.identity_plan();
        plan.skill_order[0] = vec!["C".into(), "Rust".into(), "Go".into()];
        assert_eq!(doc.render(&plan), src.replace("Rust, Go, C", "C, Rust, Go"));
    }
}
