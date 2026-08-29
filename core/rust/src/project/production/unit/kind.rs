#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitKind {
    Definition,
    Macro,
    Registration,
    Initializer,
}

impl UnitKind {
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Definition => "definition",
            Self::Macro => "macro",
            Self::Registration => "registration",
            Self::Initializer => "initializer",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    Pure,
    Unknown,
    Effectful,
}

impl Effect {
    pub(crate) const fn keyword(self) -> &'static str {
        match self {
            Self::Pure => "pure",
            Self::Unknown => "unknown",
            Self::Effectful => "effectful",
        }
    }
}
