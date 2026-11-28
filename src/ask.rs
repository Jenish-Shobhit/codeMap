use crate::app::App;
use crate::git::branch::Deletion;
use crate::status::Discard;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    Discard(Discard),
    Switch {
        branch: String,
    },
    Merge {
        branch: String,
        into: String,
        agent_working: bool,
    },
    Delete(Deletion),
}

impl Ask {
    pub fn title(&self) -> &'static str {
        match self {
            Ask::Discard(_) => "discard",
            Ask::Switch { .. } => "switch",
            Ask::Merge { .. } => "merge",
            Ask::Delete(_) => "delete",
        }
    }

    pub fn question(&self) -> String {
        match self {
            Ask::Discard(discard) => discard.question(),
            Ask::Switch { branch } => format!("switch to {branch} while the agent is working?"),
            Ask::Merge { branch, into, .. } => format!("merge {branch} into {into}?"),
            Ask::Delete(deletion) => format!("delete the branch {}?", deletion.branch),
        }
    }

    pub fn caution(&self) -> Option<String> {
        match self {
            Ask::Discard(_) => Some("this cannot be undone".into()),
            Ask::Switch { .. } => {
                Some("the agent is working here: switching moves the files under it".into())
            }
            Ask::Merge { agent_working, .. } => agent_working
                .then(|| "the agent is working here: merging changes the files under it".into()),
            Ask::Delete(deletion) => match deletion.only_here {
                0 => None,
                1 => {
                    Some("1 commit exists only on this branch and would become unreachable".into())
                }
                many => Some(format!(
                    "{many} commits exist only on this branch and would become unreachable"
                )),
            },
        }
    }

    pub fn verb(&self) -> &'static str {
        match self {
            Ask::Discard(discard) => discard.verb(),
            Ask::Switch { .. } => "switches",
            Ask::Merge { .. } => "merges",
            Ask::Delete(_) => "deletes",
        }
    }

    pub fn declined(&self) -> &'static str {
        match self {
            Ask::Discard(_) => "nothing discarded",
            Ask::Switch { .. } => "nothing switched",
            Ask::Merge { .. } => "nothing merged",
            Ask::Delete(_) => "nothing deleted",
        }
    }
}

impl App {
    pub fn answer(&mut self, yes: bool) {
        let Some(asked) = self.confirm.take() else {
            return;
        };
        if !yes {
            self.message = Some(asked.declined().into());
            return;
        }
        match asked {
            Ask::Discard(discard) => self.discard(&discard),
            Ask::Switch { branch } => self.switch_to(&branch),
            Ask::Merge { branch, .. } => self.merge(&branch),
            Ask::Delete(deletion) => self.delete(&deletion),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn deleting(only_here: usize) -> Ask {
        Ask::Delete(Deletion {
            branch: "feat/x".into(),
            tip: "a384f27".into(),
            only_here,
        })
    }

    #[test]
    fn a_delete_question_says_what_would_be_lost() {
        let safe = deleting(0);
        assert_eq!(safe.question(), "delete the branch feat/x?");
        assert_eq!(safe.caution(), None);
        assert_eq!((safe.title(), safe.verb()), ("delete", "deletes"));
        assert_eq!(
            deleting(1).caution().as_deref(),
            Some("1 commit exists only on this branch and would become unreachable")
        );
        assert_eq!(
            deleting(12).caution().as_deref(),
            Some("12 commits exist only on this branch and would become unreachable")
        );
        assert_eq!(deleting(12).declined(), "nothing deleted");
    }

    #[test]
    fn a_merge_question_names_both_branches() {
        let merge = |agent_working| Ask::Merge {
            branch: "feat/x".into(),
            into: "main".into(),
            agent_working,
        };
        assert_eq!(merge(false).question(), "merge feat/x into main?");
        assert_eq!(merge(false).caution(), None);
        assert!(merge(true).caution().unwrap().contains("agent is working"));
        assert_eq!(
            (merge(false).title(), merge(false).verb()),
            ("merge", "merges")
        );
        assert_eq!(merge(true).declined(), "nothing merged");
    }

    #[test]
    fn a_switch_question_says_the_agent_is_working() {
        let switch = Ask::Switch {
            branch: "feat/x".into(),
        };
        assert_eq!(
            switch.question(),
            "switch to feat/x while the agent is working?"
        );
        assert_eq!(
            switch.caution().as_deref(),
            Some("the agent is working here: switching moves the files under it")
        );
        assert_eq!(
            (switch.title(), switch.verb(), switch.declined()),
            ("switch", "switches", "nothing switched")
        );
    }
}
