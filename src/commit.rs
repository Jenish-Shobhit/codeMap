#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Field {
    #[default]
    Subject,
    Body,
}

pub const EMPTY_SUBJECT: &str = "an empty subject does not commit · write one first";
const PROBLEM_LINES: usize = 4;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CommitBox {
    pub open: bool,
    pub subject: String,
    pub body: String,
    pub field: Field,
    pub busy: bool,
    pub problem: Vec<String>,
}

fn typed(c: char) -> bool {
    !c.is_control()
}

impl CommitBox {
    fn text(&mut self) -> &mut String {
        match self.field {
            Field::Subject => &mut self.subject,
            Field::Body => &mut self.body,
        }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.busy = false;
        self.problem.clear();
    }

    pub fn close(&mut self) {
        self.open = false;
        self.busy = false;
        self.problem.clear();
    }

    pub fn type_char(&mut self, c: char) {
        if typed(c) {
            self.text().push(c);
            self.problem.clear();
        }
    }

    pub fn paste(&mut self, pasted: &str) {
        let lines = pasted.replace("\r\n", "\n").replace('\r', "\n");
        let kept: String = match self.field {
            Field::Subject => lines
                .split('\n')
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>()
                .join(" "),
            Field::Body => lines,
        };
        let body = self.field == Field::Body;
        self.text()
            .extend(kept.chars().filter(|c| typed(*c) || (body && *c == '\n')));
        self.problem.clear();
    }

    pub fn backspace(&mut self) {
        self.text().pop();
        self.problem.clear();
    }

    pub fn clear_field(&mut self) {
        self.text().clear();
        self.problem.clear();
    }

    pub fn new_line(&mut self) {
        if self.field == Field::Body {
            self.body.push('\n');
        }
    }

    pub fn other_field(&mut self) {
        self.field = match self.field {
            Field::Subject => Field::Body,
            Field::Body => Field::Subject,
        };
    }

    pub fn message(&self) -> Option<(String, String)> {
        let subject = self.subject.trim();
        (!subject.is_empty()).then(|| (subject.to_string(), self.body.trim_end().to_string()))
    }

    pub fn refuse(&mut self, why: &str) {
        self.busy = false;
        self.problem = why
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty())
            .take(PROBLEM_LINES)
            .map(str::to_string)
            .collect();
    }

    pub fn committed(&mut self) {
        *self = CommitBox::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typing(text: &str) -> CommitBox {
        let mut b = CommitBox::default();
        b.open();
        for c in text.chars() {
            b.type_char(c);
        }
        b
    }

    #[test]
    fn typing_goes_to_the_subject_until_tab_moves_to_the_body() {
        let mut b = typing("fix: the loop");
        assert_eq!((b.subject.as_str(), b.body.as_str()), ("fix: the loop", ""));
        b.new_line();
        assert_eq!(b.subject, "fix: the loop", "the subject is one line");
        b.other_field();
        assert_eq!(b.field, Field::Body);
        b.type_char('a');
        b.new_line();
        b.type_char('b');
        assert_eq!(b.body, "a\nb");
        b.backspace();
        b.backspace();
        assert_eq!(b.body, "a", "backspace joins the lines again");
        b.other_field();
        b.backspace();
        assert_eq!(b.subject, "fix: the loo");
    }

    #[test]
    fn clearing_empties_the_field_the_cursor_is_in() {
        let mut b = typing("subject");
        b.other_field();
        b.type_char('x');
        b.clear_field();
        assert_eq!((b.subject.as_str(), b.body.as_str()), ("subject", ""));
        b.other_field();
        b.clear_field();
        assert_eq!(b.subject, "");
        b.backspace();
        assert_eq!(b.subject, "");
    }

    #[test]
    fn closing_keeps_the_draft_and_a_commit_clears_it() {
        let mut b = typing("subject");
        b.other_field();
        b.type_char('x');
        b.refuse("hook said no");
        b.close();
        assert!(!b.open && b.problem.is_empty());
        b.open();
        assert_eq!(
            (b.subject.as_str(), b.body.as_str(), b.field),
            ("subject", "x", Field::Body)
        );
        b.committed();
        assert_eq!(b, CommitBox::default());
    }

    #[test]
    fn the_message_is_the_trimmed_subject_and_the_body() {
        assert_eq!(typing("   ").message(), None);
        let mut b = typing("  fix: trim me ");
        b.other_field();
        b.paste("line one\r\n  indented\n\n\n");
        assert_eq!(
            b.message(),
            Some((
                "fix: trim me".to_string(),
                "line one\n  indented".to_string()
            ))
        );
    }

    #[test]
    fn a_problem_shows_its_first_lines_until_the_next_edit() {
        let mut b = typing("subject");
        b.busy = true;
        b.refuse("\nAuthor identity unknown\n\n*** Please tell me who you are.\n\nRun\n  a\n  b\n");
        assert!(!b.busy);
        assert_eq!(
            b.problem,
            [
                "Author identity unknown",
                "*** Please tell me who you are.",
                "Run",
                "  a"
            ]
        );
        b.type_char('!');
        assert!(b.problem.is_empty());
        assert_eq!(b.subject, "subject!");
    }

    #[test]
    fn pasted_text_is_text_and_a_pasted_subject_stays_on_one_line() {
        let mut b = typing("");
        b.paste("fix: one\n\n  two\u{1b}[31m\tthree\r\n");
        assert_eq!(b.subject, "fix: one two[31mthree");
        b.other_field();
        b.paste("a\u{7}b\r\nc\rd");
        assert_eq!(b.body, "ab\nc\nd");
        b.type_char('\n');
        b.type_char('\u{1b}');
        assert_eq!(b.body, "ab\nc\nd", "control characters are not typed");
    }
}
