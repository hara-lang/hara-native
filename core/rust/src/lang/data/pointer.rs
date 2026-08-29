use crate::core::Value;
use crate::lang::data::{Keyword, Map, Metadata};
use crate::lang::hash::JavaHash;
use crate::lang::protocol::{HashType, IDisplay, IHash, IMetadata, IObjType, ObjType};
use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;

/// An immutable, context-qualified reference descriptor.
///
/// Pointers deliberately contain no runtime, resolver, target, or dereferenced
/// value. Resolution is owned by the active evaluator context.
#[derive(Debug, Clone)]
pub struct Pointer {
    context: Keyword,
    fields: Map<Value, Value>,
    metadata: Option<Rc<Metadata>>,
}

impl Pointer {
    pub fn new(context: Keyword, fields: Map<Value, Value>) -> Self {
        Self {
            context,
            fields,
            metadata: None,
        }
    }

    pub fn context(&self) -> &Keyword {
        &self.context
    }

    pub fn fields(&self) -> &Map<Value, Value> {
        &self.fields
    }

    pub fn get(&self, key: &Value) -> Option<&Value> {
        self.fields.get(key)
    }

    pub fn descriptor(&self) -> Map<Value, Value> {
        self.fields.assoc_value(
            Value::Keyword(Keyword::from("context")),
            Value::Keyword(self.context.clone()),
        )
    }
}

impl PartialEq for Pointer {
    fn eq(&self, other: &Self) -> bool {
        self.context == other.context && self.fields == other.fields
    }
}

impl Eq for Pointer {}

impl IMetadata for Pointer {
    type Metadata = Rc<Metadata>;

    fn meta(&self) -> Option<&Self::Metadata> {
        self.metadata.as_ref()
    }

    fn with_meta(&self, metadata: Option<Self::Metadata>) -> Self {
        Self {
            context: self.context.clone(),
            fields: self.fields.clone(),
            metadata,
        }
    }
}

impl IDisplay for Pointer {
    fn display(&self) -> String {
        format!("#ptr {}", Value::Map(self.descriptor()).display())
    }
}

impl IObjType for Pointer {
    fn obj_type(&self) -> ObjType {
        ObjType::Pointer
    }
}

impl IHash for Pointer {
    fn hash_calc(&self, hash_type: HashType) -> u64 {
        crate::lang::hash::compose_ordered(
            "POINTER",
            [
                self.context.java_hash(hash_type),
                self.fields.hash_calc(hash_type) as i64,
            ],
        ) as u64
    }
}

impl crate::lang::hash::JavaHash for Pointer {
    fn java_hash(&self, hash_type: HashType) -> i64 {
        self.hash_calc(hash_type) as i64
    }
}

impl Hash for Pointer {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.hash_calc(crate::lang::hash::DEFAULT_HASH));
    }
}

impl fmt::Display for Pointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pointer_identity_is_structural_and_display_is_literal() {
        let fields: Map<Value, Value> = vec![(
            Value::Keyword(Keyword::from("id")),
            Value::String("ROOT".into()),
        )]
        .into_iter()
        .collect();
        let left = Pointer::new(Keyword::from("kernel"), fields.clone());
        let right = Pointer::new(Keyword::from("kernel"), fields);
        assert_eq!(left, right);
        assert_eq!(
            left.hash_calc(HashType::Rapid),
            right.hash_calc(HashType::Rapid)
        );
        assert!(left.display().starts_with("#ptr {"));
        assert!(left.display().contains(":context :kernel"));
        assert!(left.display().contains(":id \"ROOT\""));
    }
}
