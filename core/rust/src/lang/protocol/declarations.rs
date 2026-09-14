#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolAvailability {
    Portable,
    CapabilityGated,
    InventoryOnly,
}

impl ProtocolAvailability {
    pub fn is_guest_visible(self) -> bool {
        !matches!(self, Self::InventoryOnly)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolArity {
    Fixed(usize),
    Variadic {
        minimum: usize,
        maximum: Option<usize>,
    },
}

impl ProtocolArity {
    pub fn guest_arity(self) -> usize {
        match self {
            Self::Fixed(arity) => arity,
            Self::Variadic { .. } => usize::MAX,
        }
    }

    pub fn range(self) -> (usize, Option<usize>) {
        match self {
            Self::Fixed(arity) => (arity, Some(arity)),
            Self::Variadic { minimum, maximum } => (minimum, maximum),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolMethodDeclaration {
    pub name: &'static str,
    pub rust_name: &'static str,
    pub arity: ProtocolArity,
    pub whole_wasm: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolDeclaration {
    pub namespace: &'static str,
    pub name: &'static str,
    pub parents: &'static [&'static str],
    pub availability: ProtocolAvailability,
    pub capability: Option<&'static str>,
    pub methods: &'static [ProtocolMethodDeclaration],
}

impl ProtocolDeclaration {
    pub fn qualified_name(self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }

    pub fn runtime_name(self) -> String {
        format!("{}.{}", self.namespace, self.name)
    }

    pub fn method(self, name: &str) -> Option<ProtocolMethodDeclaration> {
        self.methods
            .iter()
            .copied()
            .find(|method| method.name == name)
    }
}

inventory::collect!(ProtocolDeclaration);

static PROTOCOL_DECLARATIONS: std::sync::OnceLock<Vec<ProtocolDeclaration>> =
    std::sync::OnceLock::new();

pub fn protocol_declarations() -> &'static [ProtocolDeclaration] {
    PROTOCOL_DECLARATIONS
        .get_or_init(|| {
            let mut declarations = inventory::iter::<ProtocolDeclaration>
                .into_iter()
                .copied()
                .collect::<Vec<_>>();
            declarations.sort_by_key(|declaration| (declaration.namespace, declaration.name));
            declarations
        })
        .as_slice()
}

pub fn find_protocol(name: &str) -> Option<ProtocolDeclaration> {
    protocol_declarations().iter().copied().find(|protocol| {
        protocol.name == name
            || name.strip_prefix(protocol.namespace).is_some_and(|suffix| {
                suffix
                    .strip_prefix('/')
                    .or_else(|| suffix.strip_prefix('.'))
                    == Some(protocol.name)
            })
    })
}

#[cfg(test)]
mod lookup_tests {
    use super::*;

    #[test]
    fn protocol_lookup_preserves_all_registered_spellings() {
        for declaration in protocol_declarations() {
            for spelling in [
                declaration.name.to_owned(),
                declaration.qualified_name(),
                declaration.runtime_name(),
            ] {
                assert_eq!(find_protocol(&spelling), Some(*declaration), "{spelling}");
                assert_eq!(find_protocol(&format!("{spelling}extra")), None);
                assert_eq!(find_protocol(&format!("extra{spelling}")), None);
            }
            for separator in ["", ":", "//", "..", "/."] {
                let spelling = format!("{}{separator}{}", declaration.namespace, declaration.name);
                assert_eq!(find_protocol(&spelling), None, "{spelling}");
            }
        }
        assert_eq!(find_protocol(""), None);
        assert_eq!(find_protocol("std.protocol"), None);
    }

    #[test]
    #[ignore = "manual fixed-work lookup timing; no timing threshold"]
    fn protocol_lookup_fixed_work_timing() {
        let names = protocol_declarations()
            .iter()
            .map(|declaration| declaration.runtime_name())
            .collect::<Vec<_>>();
        let start = std::time::Instant::now();
        for _ in 0..100 {
            for name in &names {
                assert!(std::hint::black_box(find_protocol(std::hint::black_box(name))).is_some());
            }
        }
        eprintln!("{} registered-name lookups: {:?}", names.len() * 100, start.elapsed());
    }
}
