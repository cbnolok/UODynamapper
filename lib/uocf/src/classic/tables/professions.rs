//! `prof.txt`, the professions of the character creation, read as the 2024 Classic client reads it, strictly,
//! and written back byte for byte.
//!
//! Traced in the 2024 Classic client (`UOClassicDataFiles::Professions_Load`,
//! `scripts/dev-helpers/ec-research/ghidra-classic-text-tables`):
//!
//! - The file is opened in text mode and read with `fgets(buffer, 255)` ([`MAX_LINE_BYTES`]).
//! - A line holding `Begin` anywhere starts a profession; else a line holding `End` anywhere adds it. Any other
//!   line is cut with `strtok` at spaces, tabs and line ends; the first word is a keyword, compared without case:
//!   - `Name` (the next word; quotes separate words too) into 128 bytes;
//!   - `TrueName`: **the text from the byte after the keyword to the next quote**, into 128 bytes; the rest is
//!     not read. The shipped `TrueName\t\t"Samurai"` therefore reads as a tab; `TrueName\t"Samurai"` would read
//!     as `Samurai`;
//!   - `Desc`, `NameId`, `DescId`, `Gump`: a number (`atol`);
//!   - `TopLevel`: true when the word is `true`;
//!   - `Type`: a category when the word is `Category` (the shipped files write `Profession`);
//!   - `Children`: names separated by spaces, tabs, `,`, `.` or quotes;
//!   - `Skill NAME VALUE`: the next word is the skill name ([`SKILL_NAMES`]; any other name is skill -1), the
//!     word after it the value (`atol`, kept in 16 bits); the rest is not read. **The 7.0.20 and later files'
//!     `Spirit Speak` and `Evaluate Intelligence` therefore read as skill -1 with the value 0** (`atol("Speak")`):
//!     they are kept as the client reads them ([`ProfessionLine::Skill`]);
//!   - `Stat Str|Dex|Int VALUE`;
//!   - any other keyword is ignored.
//!
//! Strict where the client is lenient or fails: a keyword line outside `Begin` ... `End` (the client writes
//! through a null profession), `End` without `Begin`, `Begin` before the open profession's `End` and a profession
//! still open at the end of the file (the client drops it), a missing or non-numeric value (`atol` reads 0), an
//! unknown keyword or stat, a word after the value (but for `TrueName` and `Skill`), and a name longer than the client's buffer are errors
//! with the line and column ([`DefSyntaxError`]).
//!
//! The file is kept line by line in a [`DefDocument`] ([`ProfessionsFile::document`]); an edited or added line
//! is written as [`ProfessionLine`]'s `format` writes it (a `TrueName` with one tab, so the client reads the name).

crate::eyre_imports!();
use std::path::Path;

use super::def_files::{DefDocument, DefLine, DefRecord, DefSyntaxError};
pub use super::def_files::MAX_LINE_BYTES;

pub const PROFESSIONS_FILE: &str = "prof.txt";
/// Bytes of the client's buffers for `Name` and `TrueName`, their terminating NUL among them.
pub const NAME_BUFFER_BYTES: usize = 128;
/// The skill names the client knows, by skill id.
pub const SKILL_NAMES: [&str; 58] = [
    "Alchemy", "Anatomy", "AnimalLore", "ItemID", "ArmsLore", "Parrying", "Begging", "Blacksmith", "Bowcraft",
    "Peacemaking", "Camping", "Carpentry", "Cartography", "Cooking", "DetectHidden", "Enticement",
    "EvaluateIntelligence", "Healing", "Fishing", "ForensicEvaluation", "Herding", "Hiding", "Provocation",
    "Inscription", "Lockpicking", "Magery", "ResistingSpells", "Tactics", "Snooping", "Musicianship", "Poisoning",
    "Archery", "SpiritSpeak", "Stealing", "Tailoring", "AnimalTaming", "TasteIdentification", "Tinkering",
    "Tracking", "Veterinary", "Swordsmanship", "MaceFighting", "Fencing", "Wrestling", "Lumberjacking", "Mining",
    "Meditation", "Stealth", "Disarm", "Necromancy", "Focus", "Chivalry", "Bushido", "Ninjitsu", "Spellweaving",
    "Mysticism", "Imbuing", "Throwing",
];

const BEGIN: &str = "Begin";
const END: &str = "End";
const TRUE_WORD: &str = "true";
const FALSE_WORD: &str = "false";
const CATEGORY_WORD: &str = "Category";
const QUOTE: char = '"';
/// What `strtok` cuts a line at: the keyword, `Gump` and `Type`.
const WORD_SEPARATORS: &[char] = &[' ', '\t', '\n', '\r'];
/// ... the values of `Name`, `Desc`, `NameId`, `DescId`, `TopLevel`.
const VALUE_SEPARATORS: &[char] = &[' ', '\t', '\n', '\r', '"'];
/// ... the words of `Children`, `Skill`, `Stat`.
const LIST_SEPARATORS: &[char] = &[' ', ',', '.', '\t', '\n', '\r', '"'];

/// A stat of `Stat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stat {
    Str,
    Dex,
    Int,
}

impl Stat {
    const ALL: [Stat; 3] = [Stat::Str, Stat::Dex, Stat::Int];

    pub fn name(self) -> &'static str {
        match self {
            Self::Str => "Str",
            Self::Dex => "Dex",
            Self::Int => "Int",
        }
    }
}

/// A line of `prof.txt` that is not blank.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfessionLine {
    Begin,
    End,
    Name(String),
    /// What the client reads: up to the next quote (see the module notes).
    TrueName(String),
    Desc(i32),
    NameId(i32),
    DescId(i32),
    TopLevel(bool),
    Gump(i32),
    /// The word; [`Profession::category`] is whether it is `Category`.
    Type(String),
    Children(Vec<String>),
    /// The skill name as the client cuts it (its id: [`ProfessionLine::skill_id`]) and the value as `atol` reads
    /// the next word, in 16 bits.
    Skill { name: String, value: u16 },
    Stat { stat: Stat, value: i16 },
}

/// `strtok` over one line: the words and where each starts.
struct Tokens<'a> {
    line: &'a str,
    at: usize,
    line_number: usize,
}

impl<'a> Tokens<'a> {
    fn error(&self, at: usize, message: impl Into<String>) -> DefSyntaxError {
        DefSyntaxError { line: self.line_number, column: self.line[..at].chars().count() + 1, message: message.into() }
    }

    /// The next word cut at `separators`, and its byte offset; like `strtok`, the byte after it is consumed.
    fn next(&mut self, separators: &[char]) -> Option<(usize, &'a str)> {
        let rest = &self.line[self.at..];
        let start = self.at + rest.find(|c: char| !separators.contains(&c))?;
        let length = self.line[start..].find(|c: char| separators.contains(&c)).unwrap_or(self.line.len() - start);
        let end = start + length;
        self.at = self.line[end..].chars().next().map_or(end, |c| end + c.len_utf8());
        Some((start, &self.line[start..end]))
    }

    fn required(&mut self, separators: &[char], what: &str) -> Result<(usize, &'a str), DefSyntaxError> {
        let at = self.line.len();
        self.next(separators).ok_or_else(|| self.error(at, format!("expected {what}")))
    }

    fn number<T: TryFrom<i64>>(&mut self, separators: &[char], what: &str) -> Result<T, DefSyntaxError> {
        let (at, word) = self.required(separators, what)?;
        let value: i64 = word.parse().map_err(|_| self.error(at, format!("{what} `{word}` is not a number (the client reads it with atol)")))?;
        T::try_from(value).map_err(|_| self.error(at, format!("{what} {value} is out of range")))
    }

    fn name(&self, at: usize, name: &str, what: &str) -> Result<String, DefSyntaxError> {
        // A character is a byte: the file is read as Latin-1.
        if name.chars().count() >= NAME_BUFFER_BYTES {
            return Err(self.error(at, format!("{what} is longer than the client's {NAME_BUFFER_BYTES}-byte buffer")));
        }
        Ok(name.to_owned())
    }

    /// Nothing but separators may follow.
    fn finish(&mut self) -> Result<(), DefSyntaxError> {
        match self.next(LIST_SEPARATORS) {
            Some((at, word)) => Err(self.error(at, format!("`{word}` after the value is not read by the client"))),
            None => Ok(()),
        }
    }
}

impl DefRecord for ProfessionLine {
    fn parse(line: &str, line_number: usize) -> Result<Option<Self>, DefSyntaxError> {
        if line.contains(BEGIN) {
            return Ok(Some(Self::Begin));
        }
        if line.contains(END) {
            return Ok(Some(Self::End));
        }
        let mut tokens = Tokens { line, at: 0, line_number };
        let Some((keyword_at, keyword)) = tokens.next(WORD_SEPARATORS) else { return Ok(None) };
        let parsed = match keyword.to_ascii_lowercase().as_str() {
            "name" => {
                let (at, name) = tokens.required(VALUE_SEPARATORS, "the name")?;
                Self::Name(tokens.name(at, name, "the name")?)
            }
            "truename" => {
                // strtok(NULL, "\""): from the byte after the keyword's separator to the next quote.
                let rest = &line[tokens.at..];
                let start = tokens.at + rest.find(|c: char| c != QUOTE).ok_or_else(|| tokens.error(line.len(), "expected the true name"))?;
                let length = line[start..].find(QUOTE).ok_or_else(|| {
                    tokens.error(start, "the true name has no closing quote (the client would keep the line end in it)")
                })?;
                // What follows is not read; in the shipped files it is the quoted name itself.
                return Ok(Some(Self::TrueName(tokens.name(start, &line[start..start + length], "the true name")?)));
            }
            "desc" => Self::Desc(tokens.number(VALUE_SEPARATORS, "the description")?),
            "nameid" => Self::NameId(tokens.number(VALUE_SEPARATORS, "the name cliloc")?),
            "descid" => Self::DescId(tokens.number(VALUE_SEPARATORS, "the description cliloc")?),
            "toplevel" => {
                let (at, word) = tokens.required(VALUE_SEPARATORS, "`true` or `false`")?;
                match word.to_ascii_lowercase().as_str() {
                    TRUE_WORD => Self::TopLevel(true),
                    FALSE_WORD => Self::TopLevel(false),
                    _ => return Err(tokens.error(at, format!("`{word}` is not `true` or `false`"))),
                }
            }
            "gump" => Self::Gump(tokens.number(WORD_SEPARATORS, "the gump")?),
            "type" => Self::Type(tokens.required(WORD_SEPARATORS, "the type")?.1.to_owned()),
            "children" => {
                let mut children = Vec::new();
                while let Some((at, child)) = tokens.next(LIST_SEPARATORS) {
                    children.push(tokens.name(at, child, "a child")?);
                }
                Self::Children(children)
            }
            "skill" => {
                let (at, name) = tokens.required(LIST_SEPARATORS, "a skill name")?;
                let name = tokens.name(at, name, "the skill name")?;
                let (_, value) = tokens.required(LIST_SEPARATORS, "the skill value")?;
                // As the client reads it; what follows is not read (see the module notes).
                return Ok(Some(Self::Skill { name, value: atol(value) as u16 }));
            }
            "stat" => {
                let (at, name) = tokens.required(LIST_SEPARATORS, "Str, Dex or Int")?;
                let stat = Stat::ALL
                    .into_iter()
                    .find(|stat| stat.name().eq_ignore_ascii_case(name))
                    .ok_or_else(|| tokens.error(at, format!("`{name}` is not Str, Dex or Int (the client ignores the line)")))?;
                Self::Stat { stat, value: tokens.number(LIST_SEPARATORS, "the stat value")? }
            }
            _ => return Err(tokens.error(keyword_at, format!("`{keyword}` is no keyword the client reads (it ignores the line)"))),
        };
        tokens.finish()?;
        Ok(Some(parsed))
    }

    fn format(&self) -> String {
        match self {
            Self::Begin => BEGIN.to_owned(),
            Self::End => END.to_owned(),
            Self::Name(name) => format!("\tName\t{name}"),
            Self::TrueName(name) => format!("\tTrueName\t{QUOTE}{name}{QUOTE}"),
            Self::Desc(value) => format!("\tDesc\t{value}"),
            Self::NameId(value) => format!("\tNameId\t{value}"),
            Self::DescId(value) => format!("\tDescId\t{value}"),
            Self::TopLevel(value) => format!("\tTopLevel\t{}", if *value { TRUE_WORD } else { FALSE_WORD }),
            Self::Gump(value) => format!("\tGump\t{value}"),
            Self::Type(word) => format!("\tType\t{word}"),
            Self::Children(children) => format!("\tChildren\t{}", children.join(",")),
            Self::Skill { name, value } => format!("\tSkill\t{name}\t{value}"),
            Self::Stat { stat, value } => format!("\tStat\t{}\t{value}", stat.name()),
        }
    }
}

/// A profession as the client holds it after its `End`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Profession {
    pub name: String,
    pub true_name: String,
    pub desc: i32,
    pub name_id: i32,
    pub desc_id: i32,
    pub top_level: bool,
    pub gump: i32,
    /// `Type Category`.
    pub category: bool,
    pub children: Vec<String>,
    /// Skill ids (`None` for the client's -1) and values, in file order.
    pub skills: Vec<(Option<u16>, u16)>,
    pub str: i16,
    pub dex: i16,
    pub int: i16,
}

/// `prof.txt`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfessionsFile {
    pub document: DefDocument<ProfessionLine>,
}

impl ProfessionsFile {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DefSyntaxError> {
        let file = Self { document: DefDocument::from_bytes(bytes)? };
        file.professions()?;
        Ok(file)
    }

    pub fn load(path: &Path) -> eyre::Result<Self> {
        let bytes = std::fs::read(path).wrap_err_with(|| format!("read {}", path.display()))?;
        Self::from_bytes(&bytes).wrap_err_with(|| format!("{}", path.display()))
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.document.to_bytes()
    }

    pub fn save(&self, path: &Path) -> eyre::Result<()> {
        self.professions().wrap_err_with(|| format!("{}", path.display()))?;
        self.document.save(path)
    }

    /// The professions the client adds, in file order; an error for a line out of a `Begin` ... `End` block.
    pub fn professions(&self) -> Result<Vec<Profession>, DefSyntaxError> {
        let error = |index: usize, message: &str| DefSyntaxError { line: index + 1, column: 1, message: message.to_owned() };
        let mut professions = Vec::new();
        let mut open: Option<(usize, Profession)> = None;
        for (index, line) in self.document.lines.iter().enumerate() {
            let DefLine::Record { record, .. } = line else { continue };
            match (record, open.as_mut()) {
                (ProfessionLine::Begin, Some(_)) => {
                    return Err(error(index, "`Begin` before the open profession's `End` (the client drops that profession)"));
                }
                (ProfessionLine::Begin, None) => open = Some((index, Profession::default())),
                (ProfessionLine::End, Some(_)) => professions.push(open.take().expect("open").1),
                (_, None) => return Err(error(index, "a line out of a `Begin` ... `End` block (the client has no profession for it)")),
                (record, Some((_, profession))) => apply(profession, record),
            }
        }
        if let Some((index, _)) = open {
            return Err(error(index, "a profession without `End` (the client drops it)"));
        }
        Ok(professions)
    }
}

/// The id of a skill name as the client looks it up (without case), `None` for the client's -1.
pub fn skill_id(name: &str) -> Option<u16> {
    SKILL_NAMES.iter().position(|known| known.eq_ignore_ascii_case(name)).map(|id| id as u16)
}

/// `atol`: spaces, a sign and the digits that follow; 0 without a digit, wrapped to 32 bits as the client's `long`.
fn atol(word: &str) -> i32 {
    let word = word.trim_start_matches(|c: char| c.is_ascii_whitespace());
    let (negative, digits) = match word.as_bytes().first() {
        Some(b'-') => (true, &word[1..]),
        Some(b'+') => (false, &word[1..]),
        _ => (false, word),
    };
    let magnitude = digits.bytes().take_while(u8::is_ascii_digit).fold(0i32, |value, digit| {
        value.wrapping_mul(10).wrapping_add(i32::from(digit - b'0'))
    });
    if negative { magnitude.wrapping_neg() } else { magnitude }
}

fn apply(profession: &mut Profession, line: &ProfessionLine) {
    match line {
        ProfessionLine::Begin | ProfessionLine::End => {}
        ProfessionLine::Name(name) => profession.name = name.clone(),
        ProfessionLine::TrueName(name) => profession.true_name = name.clone(),
        ProfessionLine::Desc(value) => profession.desc = *value,
        ProfessionLine::NameId(value) => profession.name_id = *value,
        ProfessionLine::DescId(value) => profession.desc_id = *value,
        // The client only ever sets it.
        ProfessionLine::TopLevel(value) => profession.top_level |= *value,
        ProfessionLine::Gump(value) => profession.gump = *value,
        ProfessionLine::Type(word) => profession.category = word.eq_ignore_ascii_case(CATEGORY_WORD),
        ProfessionLine::Children(children) => profession.children.extend(children.iter().cloned()),
        ProfessionLine::Skill { name, value } => profession.skills.push((skill_id(name), *value)),
        ProfessionLine::Stat { stat: Stat::Str, value } => profession.str = *value,
        ProfessionLine::Stat { stat: Stat::Dex, value } => profession.dex = *value,
        ProfessionLine::Stat { stat: Stat::Int, value } => profession.int = *value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMURAI: &str = "Begin\r\n\tName\t\t\tSamurai\r\n\tTrueName\t\t\"Samurai\"\r\n\tNameId\t\t\t1062948\r\n\tDesc\t\t\t6\r\n\
        \tTopLevel\t\ttrue\r\n\tGump\t\t\t5591\r\n\tType\t\t\tProfession\r\n\tSkill\t\t\tBushido\t\t\t30\r\n\
        \tSkill\t\t\tFocus\t\t30\t\t\t\t\t\r\n\tStat\t\t\tStr\t\t\t40\r\nEnd\r\n\r\n";

    #[test]
    fn professions_read_as_the_client_reads_them_and_write_back() {
        let mut file = ProfessionsFile::from_bytes(SAMURAI.as_bytes()).unwrap();
        assert_eq!(file.to_bytes(), SAMURAI.as_bytes());
        let samurai = &file.professions().unwrap()[0];
        assert_eq!((samurai.name.as_str(), samurai.true_name.as_str()), ("Samurai", "\t"));
        assert_eq!((samurai.name_id, samurai.desc, samurai.gump, samurai.top_level, samurai.category), (1062948, 6, 5591, true, false));
        assert_eq!(samurai.skills, [(Some(52), 30), (Some(50), 30)]);
        assert_eq!(samurai.str, 40);

        for line in file.document.records_mut() {
            if let ProfessionLine::TrueName(name) = line {
                *name = "Samurai".to_owned();
            }
        }
        let edited = file.to_bytes();
        assert_eq!(edited, SAMURAI.replace("\tTrueName\t\t\"Samurai\"", "\tTrueName\t\"Samurai\"").as_bytes());
        assert_eq!(ProfessionsFile::from_bytes(&edited).unwrap().professions().unwrap()[0].true_name, "Samurai");

        let spaced = ProfessionsFile::from_bytes(b"Begin\r\n\tSkill\t\t\tSpirit Speak\t30\r\nEnd\r\n").unwrap();
        assert_eq!(spaced.professions().unwrap()[0].skills, [(None, 0)]);
        assert_eq!((atol(" -12x"), atol("Speak"), atol("+7")), (-12, 0, 7));
    }

    #[test]
    fn lines_the_client_ignores_or_fails_on_are_errors() {
        let error = |text: &str| ProfessionsFile::from_bytes(text.as_bytes()).unwrap_err();
        assert_eq!(error("\tName\tX\r\n").line, 1);
        assert!(error("Begin\r\n\tSkill\tCooking\t1\r\nBegin\r\nEnd\r\n").message.contains("Begin"));
        assert!(error("Begin\r\n").message.contains("without `End`"));
        let unknown_stat = error("Begin\r\n\tStat\tLuck\t30\r\nEnd\r\n");
        assert_eq!((unknown_stat.line, unknown_stat.column), (2, 7));
        assert!(error("Begin\r\n\tSkill\tCooking\r\nEnd\r\n").message.contains("skill value"));
        assert!(error("Begin\r\n\tGump\tabc\r\nEnd\r\n").message.contains("atol"));
        assert!(error("Begin\r\n\tColor\t3\r\nEnd\r\n").message.contains("keyword"));
        assert!(error("Begin\r\n\tDesc\t3 4\r\nEnd\r\n").message.contains("after the value"));
        assert!(error("Begin\r\n\tTrueName\t\"open\r\nEnd\r\n").message.contains("closing quote"));
        assert!(error(&format!("Begin\r\n\tName\t{}\r\nEnd\r\n", "n".repeat(NAME_BUFFER_BYTES))).message.contains("buffer"));
    }
}
