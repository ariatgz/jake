//! Finds the skills a job posting asks for and weighs them by how strongly
//! they're asked for ("required" vs "nice to have").

use std::collections::{HashMap, HashSet};

/// A skill the bot knows how to spot. `aliases` are matched case-insensitively,
/// `exact` case-sensitively (for names like "Go" or "R" that are also words).
pub struct Skill {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub exact: &'static [&'static str],
}

macro_rules! skills {
    ($( $name:literal [ $($alias:literal),* ] $( exact [ $($exact:literal),* ] )? ;)*) => {
        &[$( Skill {
            name: $name,
            aliases: &[$($alias),*],
            exact: &[$($($exact),*)?],
        } ),*]
    };
}

/// Hardware, firmware and software skills commonly listed in internship postings.
/// The name itself is matched case-insensitively unless the skill has `exact` forms.
pub static DICTIONARY: &[Skill] = skills! {
    // Languages
    "C" ["embedded c", "ansi c"];
    "C++" ["cpp", "c++11", "c++14", "c++17", "c++20", "modern c++"];
    "C#" [];
    "Python" ["python3"];
    "Java" [];
    "JavaScript" ["ecmascript"];
    "TypeScript" [];
    "Rust" [];
    "Go" ["golang"] exact ["Go"];
    "R" [] exact ["R"];
    "Kotlin" [];
    "Swift" [];
    "Scala" [];
    "Haskell" [];
    "OCaml" [];
    "SQL" [];
    "Bash" ["shell scripting", "shell script", "shell scripts"];
    "Perl" [];
    "Tcl" ["tcl/tk"];
    "MATLAB" [];
    "Simulink" [];
    "Assembly" ["assembly language"];
    "Verilog" [];
    "SystemVerilog" ["system verilog"];
    "VHDL" [];
    "Verilog-A" ["veriloga"];
    "Chisel" [];
    "SystemC" [];
    "CUDA" [];
    "OpenCL" [];
    "HTML" [];
    "CSS" [];
    "LabVIEW" [];
    // Hardware design and verification
    "RTL" ["rtl design", "register transfer level"];
    "FPGA" ["fpgas"];
    "ASIC" ["asics"];
    "SoC" ["system on chip", "system-on-chip", "socs"];
    "UVM" ["universal verification methodology"];
    "Formal verification" ["formal methods", "jaspergold"];
    "Static timing analysis" ["timing closure", "primetime"];
    "Synthesis" ["logic synthesis", "design compiler"];
    "DFT" ["design for test", "design-for-test", "scan insertion"];
    "Physical design" ["place and route", "place & route", "floorplanning", "innovus"];
    "HLS" ["high-level synthesis", "high level synthesis"];
    "Computer architecture" ["microarchitecture", "micro-architecture", "cpu architecture"];
    "Digital design" ["digital logic", "digital circuit design", "digital circuits"];
    "Analog design" ["analog circuit", "analog circuits", "analog ic"];
    "Mixed-signal" ["mixed signal"];
    "Signal integrity" ["power integrity"];
    "Power electronics" ["power supply design", "dc-dc", "power converters"];
    "PCB design" ["pcb", "pcbs", "pcba", "pcbas", "printed circuit board", "board design", "schematic capture", "pcb layout"];
    "DSP" ["digital signal processing", "signal processing"];
    "RF" ["radio frequency", "rf design"];
    "Vivado" ["vitis"];
    "Quartus" [];
    "ModelSim" ["questasim", "questa"];
    "Cadence" ["virtuoso", "allegro", "orcad", "xcelium"];
    "Synopsys" ["vcs", "verdi", "spyglass"];
    "SPICE" ["ltspice", "hspice", "pspice", "spectre"];
    "Altium" ["altium designer"];
    "KiCad" [];
    "Lab equipment" ["oscilloscope", "oscilloscopes", "logic analyzer", "logic analyzers", "multimeter", "spectrum analyzer"];
    "Soldering" [];
    "Post-silicon validation" ["post-silicon", "silicon validation", "bring-up", "bringup", "board bring-up"];
    "Testbench" ["testbenches", "test bench", "test benches"];
    // Embedded and systems
    "Embedded systems" ["embedded software", "embedded development", "embedded firmware", "embedded"];
    "Firmware" [];
    "Microcontrollers" ["microcontroller", "mcu", "mcus", "stm32", "cortex-m", "msp430", "esp32", "nrf52"];
    "RTOS" ["freertos", "zephyr", "real-time operating system", "real time operating system", "threadx"];
    "Embedded Linux" ["yocto", "buildroot", "u-boot"];
    "Linux kernel" ["kernel development", "device drivers", "device driver", "kernel modules"];
    "Linux" ["unix"];
    "Bare metal" ["bare-metal", "baremetal"];
    "Bootloader" ["bootloaders"];
    "ARM" ["arm architecture", "aarch64", "armv8"];
    "RISC-V" ["riscv"];
    "x86" [];
    "I2C" ["i²c"];
    "SPI" [];
    "UART" ["serial communication"];
    "CAN bus" ["can bus", "canbus", "can-fd"] exact ["CAN"];
    "USB" [];
    "PCIe" ["pci express"];
    "Ethernet" [];
    "DDR" ["lpddr", "ddr4", "ddr5", "dram"];
    "Bluetooth" ["ble", "bluetooth low energy"];
    "JTAG" ["swd"];
    "GDB" [];
    "Arduino" [];
    "Raspberry Pi" [];
    "ROS" ["ros2"];
    "Control systems" ["control theory", "pid control", "controls"];
    "Sensors" ["sensor integration", "sensor fusion"];
    "Motor control" ["motor drives", "bldc"];
    "Operating systems" ["os internals", "operating system"];
    "Networking" ["tcp/ip", "computer networks", "network protocols"];
    // Software and tooling
    "Git" ["github", "gitlab", "version control"];
    "CI/CD" ["continuous integration", "jenkins", "github actions", "gitlab ci"];
    "Docker" ["containers", "containerization"];
    "Kubernetes" ["k8s"];
    "AWS" ["amazon web services"];
    "GCP" ["google cloud", "google cloud platform"];
    "Azure" [];
    "CMake" ["makefile", "makefiles"];
    "Bazel" [];
    "Unit testing" ["unit tests", "test automation", "pytest", "googletest", "gtest", "junit"];
    "Debugging" ["troubleshooting", "root cause", "root-cause"];
    "REST APIs" ["rest api", "rest apis", "restful"];
    "GraphQL" [];
    "React" ["react.js", "reactjs"];
    "Node.js" ["nodejs"];
    "Django" [];
    "Flask" [];
    "FastAPI" [];
    "Spring Boot" [];
    ".NET" ["dotnet"];
    "PostgreSQL" ["postgres"];
    "MySQL" [];
    "MongoDB" [];
    "Redis" [];
    "Kafka" [];
    "Spark" ["pyspark", "apache spark"];
    "Distributed systems" ["distributed computing"];
    "Data structures and algorithms" ["data structures", "algorithms"];
    "Object-oriented programming" ["oop", "object oriented", "object-oriented"];
    "Multithreading" ["concurrency", "multi-threading", "parallel programming"];
    "Performance optimization" ["profiling", "performance tuning", "low latency", "low-latency"];
    "Machine learning" ["deep learning", "neural networks"];
    "PyTorch" [];
    "TensorFlow" [];
    "Computer vision" ["opencv"];
    "NumPy" [];
    "pandas" [];
    "Data analysis" ["data analytics", "data visualization"];
    "Excel" ["vba"];
    "Agile" ["scrum", "jira"];
    // Mechanical / general engineering tools
    "SolidWorks" [];
    "AutoCAD" [];
    "3D printing" ["additive manufacturing"];
};

/// Phrases that introduce the "must have" part of a posting.
const REQUIRED_MARKERS: &[&str] = &[
    "minimum qualifications",
    "basic qualifications",
    "required qualifications",
    "required skills",
    "requirements",
    "what you'll need",
    "what you need",
    "what we're looking for",
    "what we are looking for",
    "must have",
    "must-have",
    "you have",
    "you should have",
    "qualifications",
    "who you are",
];

/// Phrases that introduce the "nice to have" part.
const PREFERRED_MARKERS: &[&str] = &[
    "preferred qualifications",
    "preferred skills",
    "preferred experience",
    "nice to have",
    "nice-to-have",
    "bonus points",
    "ways to stand out",
    "would be an asset",
    "assets:",
    "additional qualifications",
    "pluses",
];

/// Words near a mention that downgrade it to "nice to have".
const SOFT_CUES: &[&str] = &["a plus", "an asset", "preferred", "bonus", "nice to have", "desirable", "familiarity", "exposure to"];
/// Words near a mention that upgrade it to "must have".
const HARD_CUES: &[&str] = &["required", "must", "proficien", "strong", "solid", "expert", "experience with", "experience in"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Importance {
    Required,
    Mentioned,
    Preferred,
}

#[derive(Debug, Clone)]
pub struct PostingSkill {
    pub name: String,
    pub importance: Importance,
    pub weight: f32,
}

/// A term to look for: a dictionary skill or a skill the user listed on their resume.
#[derive(Debug, Clone)]
pub struct Term {
    pub name: String,
    aliases: Vec<String>,
    exact: Vec<String>,
}

impl Term {
    fn from_skill(skill: &Skill) -> Self {
        let mut aliases = Vec::new();
        if skill.exact.is_empty() {
            aliases.push(skill.name.to_lowercase());
        }
        aliases.extend(skill.aliases.iter().map(|a| a.to_lowercase()));
        Term { name: skill.name.to_string(), aliases, exact: skill.exact.iter().map(|e| e.to_string()).collect() }
    }

    /// Byte offsets where this term appears. `lower` must be `text.to_lowercase()`.
    fn positions(&self, text: &str, lower: &str) -> Vec<usize> {
        let mut out = Vec::new();
        for alias in &self.aliases {
            out.extend(find_all(lower, alias));
        }
        for exact in &self.exact {
            out.extend(find_all(text, exact));
        }
        out.sort_unstable();
        out.dedup();
        out
    }

    fn appears_in(&self, text: &str, lower: &str) -> bool {
        self.aliases.iter().any(|a| !find_all(lower, a).is_empty()) || self.exact.iter().any(|e| !find_all(text, e).is_empty())
    }
}

/// The dictionary plus any extra terms (e.g. skills from the user's resume).
/// Extra terms that are aliases of a dictionary skill fold into that skill.
#[derive(Debug, Clone)]
pub struct Vocabulary {
    terms: Vec<Term>,
}

impl Vocabulary {
    pub fn new(extra: &[String]) -> Self {
        let mut terms: Vec<Term> = DICTIONARY.iter().map(Term::from_skill).collect();
        for raw in extra {
            let trimmed = raw.trim();
            let lower = trimmed.to_lowercase();
            if lower.chars().count() < 2 || lower.len() > 40 {
                continue;
            }
            let known = terms.iter().any(|t| t.name.to_lowercase() == lower || t.aliases.contains(&lower) || t.exact.iter().any(|e| e == trimmed));
            if !known {
                terms.push(Term { name: trimmed.to_string(), aliases: vec![lower], exact: Vec::new() });
            }
        }
        Vocabulary { terms }
    }

    /// Canonical names of every term found in `text`.
    pub fn find(&self, text: &str) -> HashSet<String> {
        let lower = text.to_lowercase();
        self.terms.iter().filter(|t| t.appears_in(text, &lower)).map(|t| t.name.clone()).collect()
    }

    /// The canonical name for a raw skill string, if it's a known term.
    pub fn canonical(&self, raw: &str) -> Option<&str> {
        let trimmed = raw.trim();
        let lower = trimmed.to_lowercase();
        self.terms
            .iter()
            .find(|t| t.name.to_lowercase() == lower || t.aliases.contains(&lower) || t.exact.iter().any(|e| e == trimmed))
            .map(|t| t.name.as_str())
    }
}

fn is_term_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '+' | '#')
}

/// Whole-term search. Unlike plain word matching, "c" doesn't match inside
/// "c++" or "c#", "r" doesn't match "R&D", and "node" doesn't match "node.js".
fn find_all(haystack: &str, needle: &str) -> Vec<usize> {
    let mut out = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut start = 0;
    // "UVM-style" and "FPGA-based" count, but one-letter names stay strict ("C-suite").
    let hyphen_blocks = needle.chars().count() == 1;
    while let Some(pos) = haystack[start..].find(needle) {
        let begin = start + pos;
        let end = begin + needle.len();
        let before = haystack[..begin].chars().next_back();
        let after = haystack[end..].chars().next();
        let before_ok = before.map_or(true, |c| !is_term_char(c) && !matches!(c, '&' | '.') && !(hyphen_blocks && c == '-'));
        let after_ok = after.map_or(true, |c| !is_term_char(c) && c != '&' && !(hyphen_blocks && c == '-'))
            // a period is punctuation unless it continues the name (".js", ".net")
            && !(after == Some('.') && haystack[end + 1..].chars().next().is_some_and(|c| c.is_alphanumeric()));
        if before_ok && after_ok {
            out.push(begin);
        }
        start = end;
    }
    out
}

fn marker_positions(lower: &str, markers: &[&str]) -> Vec<usize> {
    let mut out: Vec<usize> = markers.iter().flat_map(|m| lower.match_indices(m).map(|(i, _)| i)).collect();
    out.sort_unstable();
    out
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// The sentence around a byte offset, capped to a short window.
fn sentence_at(lower: &str, pos: usize) -> &str {
    // Offsets from case-sensitive matches come from the original text, whose
    // byte length can differ from the lowercased copy for some characters.
    let pos = floor_boundary(lower, pos.min(lower.len()));
    let start = floor_boundary(lower, pos.saturating_sub(100));
    let end = floor_boundary(lower, (pos + 100).min(lower.len()));
    let slice = &lower[start..end];
    let rel = pos - start;
    let s = slice[..rel].rfind(['.', ';', '\n', '•']).map_or(0, |i| i + 1);
    let e = slice[rel..].find(['.', ';', '\n', '•']).map_or(slice.len(), |i| rel + i);
    &slice[s..e]
}

fn classify(lower: &str, pos: usize, required: &[usize], preferred: &[usize]) -> Importance {
    let sentence = sentence_at(lower, pos);
    if SOFT_CUES.iter().any(|c| sentence.contains(c)) {
        return Importance::Preferred;
    }
    let last_required = required.iter().rev().find(|&&p| p < pos);
    let last_preferred = preferred.iter().rev().find(|&&p| p < pos);
    let section = match (last_required, last_preferred) {
        (Some(r), Some(p)) => if p > r { Importance::Preferred } else { Importance::Required },
        (None, Some(_)) => Importance::Preferred,
        (Some(_), None) => Importance::Required,
        (None, None) => Importance::Mentioned,
    };
    if section == Importance::Mentioned && HARD_CUES.iter().any(|c| sentence.contains(c)) {
        return Importance::Required;
    }
    section
}

fn rank(importance: Importance) -> u8 {
    match importance {
        Importance::Required => 0,
        Importance::Mentioned => 1,
        Importance::Preferred => 2,
    }
}

/// Skills a posting mentions, most important first.
pub fn extract(vocab: &Vocabulary, title: &str, body: &str) -> Vec<PostingSkill> {
    let lower = body.to_lowercase();
    let title_lower = title.to_lowercase();
    let required = marker_positions(&lower, REQUIRED_MARKERS);
    let preferred = marker_positions(&lower, PREFERRED_MARKERS);

    let mut out: Vec<PostingSkill> = Vec::new();
    for term in &vocab.terms {
        let positions = term.positions(body, &lower);
        let in_title = term.appears_in(title, &title_lower);
        if positions.is_empty() && !in_title {
            continue;
        }
        // The strongest classification across all mentions wins.
        let importance = positions
            .iter()
            .map(|&p| classify(&lower, p, &required, &preferred))
            .min_by_key(|i| rank(*i))
            .unwrap_or(Importance::Required);
        let base = match importance {
            Importance::Required => 3.0,
            Importance::Mentioned => 2.0,
            Importance::Preferred => 1.0,
        };
        let repeats = (positions.len().saturating_sub(1) as f32 * 0.5).min(1.5);
        let weight = base + if in_title { 2.0 } else { 0.0 } + repeats;
        out.push(PostingSkill { name: term.name.clone(), importance, weight });
    }
    out.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| a.name.cmp(&b.name)));
    out
}

const STOPWORDS: &[&str] = &[
    "about", "above", "across", "after", "again", "against", "along", "among", "another", "around", "based", "because",
    "before", "being", "below", "between", "candidate", "candidates", "company", "could", "currently", "daily", "degree",
    "different", "during", "either", "employer", "equal", "every", "excellent", "experience", "field", "fields", "further",
    "great", "having", "however", "including", "intern", "interns", "internship", "itself", "knowledge", "least",
    "looking", "might", "opportunity", "other", "others", "people", "position", "program", "provide", "pursuing",
    "related", "salary", "several", "should", "since", "skills", "strong", "student", "students", "their", "there",
    "these", "those", "through", "under", "understanding", "university", "until", "using", "where", "which", "while",
    "within", "without", "working", "would", "years", "applicants", "benefits", "ability", "status", "regard",
];

/// Very rough stemmer so "designed", "designing" and "designs" meet at "design".
pub fn stem(word: &str) -> &str {
    for suffix in ["ing", "ed", "es", "s"] {
        if word.len() > suffix.len() + 3 {
            if let Some(stripped) = word.strip_suffix(suffix) {
                return stripped;
            }
        }
    }
    word
}

/// Distinctive words (stemmed, lowercased), used as a secondary relevance signal.
pub fn significant_words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 5 && w.chars().all(|c| c.is_alphabetic()))
        .map(|w| w.to_lowercase())
        .filter(|w| !STOPWORDS.contains(&w.as_str()))
        .map(|w| stem(&w).to_string())
        .collect()
}

/// Adjacent pairs of content words ("battery management", "test automation").
/// Shared phrases are a much stronger hint than shared single words.
pub fn significant_phrases(text: &str) -> HashSet<String> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3 && w.chars().all(|c| c.is_alphabetic()))
        .map(|w| w.to_lowercase())
        .filter(|w| !STOPWORDS.contains(&w.as_str()) && !SHORT_STOPWORDS.contains(&w.as_str()))
        .map(|w| stem(&w).to_string())
        .collect();
    words.windows(2).map(|p| format!("{} {}", p[0], p[1])).collect()
}

const SHORT_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "our", "you", "your", "are", "has", "have", "this", "that", "from", "will", "its", "was",
    "who", "all", "can", "any", "into", "per", "via", "such", "like", "also", "use", "used", "new", "work", "team",
];

/// How many of the given postings mention each skill; used for the gap report.
pub fn tally<'a>(postings: impl IntoIterator<Item = &'a Vec<PostingSkill>>) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for skills in postings {
        for skill in skills {
            *counts.entry(skill.name.clone()).or_default() += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_boundaries() {
        assert_eq!(find_all("c/c++ and c#", "c"), vec![0]);
        assert_eq!(find_all("c/c++ and c#", "c++"), vec![2]);
        assert!(find_all("r&d lab", "r").is_empty());
        assert!(find_all("node.js services", "node").is_empty());
        assert_eq!(find_all("we use node.js.", "node.js"), vec![7]);
        assert_eq!(find_all("written in c.", "c"), vec![11]);
        assert!(find_all("javascript", "java").is_empty());
        assert_eq!(find_all("uvm-style testbenches", "uvm"), vec![0]);
        assert_eq!(find_all("an fpga-based design", "fpga"), vec![3]);
        assert!(find_all("c-suite", "c").is_empty());
    }

    #[test]
    fn extracts_and_weighs() {
        let vocab = Vocabulary::new(&[]);
        let body = "About the role: You will build FPGA designs.\n\
            Minimum Qualifications: Experience with Verilog or SystemVerilog. Proficiency in Python.\n\
            Preferred Qualifications: Familiarity with UVM. Experience with Perl is a plus.";
        let skills = extract(&vocab, "FPGA Design Intern", body);
        let found: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        for expected in ["FPGA", "Verilog", "SystemVerilog", "Python", "UVM", "Perl"] {
            assert!(found.contains(&expected), "missing {expected} in {found:?}");
        }
        assert_eq!(skills[0].name, "FPGA", "title skills rank first");
        let get = |n: &str| skills.iter().find(|s| s.name == n).unwrap();
        assert_eq!(get("Verilog").importance, Importance::Required);
        assert_eq!(get("UVM").importance, Importance::Preferred);
        assert_eq!(get("Perl").importance, Importance::Preferred);
        assert!(get("Verilog").weight > get("UVM").weight);
    }

    #[test]
    fn exact_terms_need_matching_case() {
        let vocab = Vocabulary::new(&[]);
        assert!(vocab.find("Services written in Go.").contains("Go"));
        let casual = vocab.find("let's go to the R&D lab, it can wait");
        assert!(!casual.contains("Go"));
        assert!(!casual.contains("R"));
        assert!(!casual.contains("CAN bus"));
        // Lowercasing "İ" changes byte lengths; offsets must not be misused.
        let skills = extract(&vocab, "Intern", &format!("{} Requirements: Go and CAN.", "İ".repeat(300)));
        assert!(skills.iter().any(|s| s.name == "Go"));
    }

    #[test]
    fn extra_terms_fold_into_dictionary() {
        let vocab = Vocabulary::new(&["Postgres".into(), "Cocotb".into()]);
        assert_eq!(vocab.canonical("postgres"), Some("PostgreSQL"));
        assert!(vocab.find("testbenches in cocotb").contains("Cocotb"));
    }
}
