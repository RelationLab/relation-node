use std::{collections::HashSet, ops::Deref};

use graph::{
    components::store::EntityType,
    data::graphql::DocumentExt,
    prelude::{q, r, s, QueryExecutionError, Schema},
};
use graphql_parser::Pos;

use crate::schema::ast::ObjectCondition;

#[derive(Debug, Clone, PartialEq)]
pub struct FragmentDefinition {
    pub position: Pos,
    pub name: String,
    pub type_condition: TypeCondition,
    pub directives: Vec<Directive>,
    pub selection_set: SelectionSet,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectionSet {
    span: (Pos, Pos),
    // map object type name -> fields
    items: Vec<(String, Vec<Field>)>,
}

impl SelectionSet {
    pub fn new(span: (Pos, Pos), types: Vec<String>) -> Self {
        let items = types.into_iter().map(|name| (name, Vec::new())).collect();
        SelectionSet { span, items }
    }

    pub fn empty_from(other: &SelectionSet) -> Self {
        let items = other
            .items
            .iter()
            .map(|(name, _)| (name.clone(), Vec::new()))
            .collect();
        SelectionSet {
            span: other.span.clone(),
            items,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn single_field(&self) -> Option<&Field> {
        let mut iter = self.items.iter();
        let field = match iter.next() {
            Some((_, fields)) => {
                if fields.len() != 1 {
                    return None;
                } else {
                    &fields[0]
                }
            }
            None => return None,
        };
        for (_, fields) in iter {
            if fields.len() != 1 {
                return None;
            }
            if &fields[0] != field {
                return None;
            }
        }
        return Some(field);
    }

    pub fn fields(&self) -> impl Iterator<Item = (&str, impl Iterator<Item = &Field>)> {
        self.items
            .iter()
            .map(|(name, fields)| (name.as_str(), fields.iter()))
    }

    pub fn interior_fields(&self) -> impl Iterator<Item = (&str, impl Iterator<Item = &Field>)> {
        self.items.iter().map(|(name, fields)| {
            (
                name.as_str(),
                fields.iter().filter(|field| !field.is_leaf()),
            )
        })
    }

    pub fn fields_for(&self, obj_type: &s::ObjectType) -> impl Iterator<Item = &Field> {
        let item = self
            .items
            .iter()
            .find(|(name, _)| name == &obj_type.name)
            .expect("there is an entry for the type");
        item.1.iter()
    }

    pub fn into_fields_for(self, obj_type: &s::ObjectType) -> impl Iterator<Item = Field> {
        let item = self
            .items
            .into_iter()
            .find(|(name, _)| name == &obj_type.name)
            .expect("there is an entry for the type");
        item.1.into_iter()
    }

    pub fn push(&mut self, new_field: &Field) {
        for (_, fields) in &mut self.items {
            Self::merge_field(fields, new_field.clone());
        }
    }

    pub fn push_fields(&mut self, fields: Vec<&Field>) {
        for field in fields {
            self.push(field);
        }
    }

    pub fn merge(&mut self, other: SelectionSet, directives: Vec<Directive>) {
        for (other_name, other_fields) in other.items {
            let item = self
                .items
                .iter_mut()
                .find(|(name, _)| &other_name == name)
                .expect("all possible types are already in items");
            for mut other_field in other_fields {
                other_field.prepend_directives(directives.clone());
                Self::merge_field(&mut item.1, other_field);
            }
        }
    }

    fn merge_field(fields: &mut Vec<Field>, new_field: Field) {
        match fields
            .iter_mut()
            .find(|field| field.response_key() == new_field.response_key())
        {
            Some(_field) => todo!("merge fields"),
            None => fields.push(new_field),
        }
    }

    pub fn restrict(&mut self, type_cond: &TypeCondition) {
        self.items.retain(|(name, _)| type_cond.matches_name(name));
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Directive {
    pub position: Pos,
    pub name: String,
    pub arguments: Vec<(String, r::Value)>,
}

impl Directive {
    /// Looks up the value of an argument in a vector of (name, value) tuples.
    pub fn argument_value(&self, name: &str) -> Option<&r::Value> {
        self.arguments
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub position: Pos,
    pub alias: Option<String>,
    pub name: String,
    pub arguments: Vec<(String, r::Value)>,
    pub directives: Vec<Directive>,
    pub selection_set: SelectionSet,
}

impl Field {
    /// Returns the response key of a field, which is either its name or its alias (if there is one).
    pub fn response_key(&self) -> &str {
        self.alias
            .as_ref()
            .map(Deref::deref)
            .unwrap_or(self.name.as_str())
    }

    /// Looks up the value of an argument in a vector of (name, value) tuples.
    pub fn argument_value(&self, name: &str) -> Option<&r::Value> {
        self.arguments
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    fn prepend_directives(&mut self, mut directives: Vec<Directive>) {
        // TODO: check that the new directives don't conflict with existing
        // directives
        std::mem::swap(&mut self.directives, &mut directives);
        self.directives.extend(directives);
    }

    fn is_leaf(&self) -> bool {
        self.selection_set.is_empty()
    }
}

// TODO: Instead of cloning type names, use ObjectCondition<'a>
#[derive(Debug, Clone, PartialEq)]
pub enum TypeCondition {
    Any,
    Only(HashSet<String>),
}

impl TypeCondition {
    pub fn convert(
        schema: &Schema,
        type_cond: Option<&q::TypeCondition>,
    ) -> Result<TypeCondition, QueryExecutionError> {
        match type_cond {
            Some(q::TypeCondition::On(name)) => Self::from_name(schema, name),
            None => Ok(TypeCondition::Any),
        }
    }

    pub fn from_name(schema: &Schema, name: &str) -> Result<TypeCondition, QueryExecutionError> {
        let set = resolve_object_types(schema, name)?
            .into_iter()
            .map(|ty| ty.name().to_string())
            .collect();
        Ok(TypeCondition::Only(set))
    }

    fn matches_name(&self, name: &str) -> bool {
        match self {
            TypeCondition::Any => true,
            TypeCondition::Only(set) => set.contains(name),
        }
    }

    pub fn intersect(self, other: &TypeCondition) -> TypeCondition {
        match self {
            TypeCondition::Any => other.clone(),
            TypeCondition::Only(set) => TypeCondition::Only(
                set.into_iter()
                    .filter(|ty| other.matches_name(ty))
                    .collect(),
            ),
        }
    }
}

/// Look up the type `name` from the schema and resolve interfaces
/// and unions until we are left with a set of concrete object types
pub(crate) fn resolve_object_types<'a>(
    schema: &'a Schema,
    name: &str,
) -> Result<HashSet<ObjectCondition<'a>>, QueryExecutionError> {
    let mut set = HashSet::new();
    match schema
        .document
        .get_named_type(name)
        .ok_or_else(|| QueryExecutionError::AbstractTypeError(name.to_string()))?
    {
        s::TypeDefinition::Interface(intf) => {
            for obj_ty in &schema.types_for_interface()[&EntityType::new(intf.name.to_string())] {
                set.insert(obj_ty.into());
            }
        }
        s::TypeDefinition::Union(tys) => {
            for ty in &tys.types {
                set.extend(resolve_object_types(schema, ty)?)
            }
        }
        s::TypeDefinition::Object(ty) => {
            set.insert(ty.into());
        }
        s::TypeDefinition::Scalar(_)
        | s::TypeDefinition::Enum(_)
        | s::TypeDefinition::InputObject(_) => {
            return Err(QueryExecutionError::NamedTypeError(name.to_string()));
        }
    }
    Ok(set)
}

#[derive(Debug, Clone, PartialEq)]
pub struct InlineFragment {
    pub position: Pos,
    pub type_condition: Option<TypeCondition>,
    pub directives: Vec<Directive>,
    pub selection_set: SelectionSet,
}

#[allow(dead_code)]
mod tmp {
    use super::Field;
    use graphql_parser::Pos;
    use std::slice::Iter;

    #[derive(Debug, Clone, PartialEq)]
    pub struct SelectionSet {
        pub span: (Pos, Pos),
        // map object type name -> fields
        pub items: Vec<(String, Vec<Field>)>,
    }

    impl SelectionSet {
        fn new(span: (Pos, Pos), types: Vec<String>) -> Self {
            let items = types.into_iter().map(|name| (name, vec![])).collect();
            Self { span, items }
        }

        fn append_set(&mut self, other: SelectionSet) {
            for (name, fields) in other.items {
                let item = self.items.iter_mut().find(|item| item.0 == name).unwrap();
                let mut fields = fields
                    .into_iter()
                    .filter(|field| !item.1.contains(field))
                    .collect::<Vec<_>>();
                item.1.append(&mut fields);
            }
        }

        fn append_field(&mut self, new_field: &Field) {
            for (_, fields) in &mut self.items {
                if !fields.contains(new_field) {
                    fields.push(new_field.clone())
                }
            }
        }

        fn append_field_for(&mut self, object_type: &str, field: Field) {
            let fields = &mut self
                .items
                .iter_mut()
                .find(|(ty, _)| ty == object_type)
                .unwrap()
                .1;
            if !fields.contains(&field) {
                fields.push(field);
            }
        }

        fn iter_fields(&self) -> SelectionSetFields<'_> {
            SelectionSetFields::new(self)
        }
    }

    pub struct SelectionSetFields<'a> {
        object_types: Iter<'a, (String, Vec<Field>)>,
        fields: Option<(&'a str, Iter<'a, Field>)>,
    }

    impl<'a> SelectionSetFields<'a> {
        fn new(set: &'a SelectionSet) -> Self {
            let mut object_types = set.items.iter();
            let fields = object_types
                .next()
                .map(|(object_type, fields)| (object_type.as_str(), fields.iter()));
            Self {
                object_types,
                fields,
            }
        }
    }

    impl<'a> Iterator for SelectionSetFields<'a> {
        type Item = (&'a str, &'a Field);

        fn next(&mut self) -> Option<Self::Item> {
            match &mut self.fields {
                None => None,
                Some((object_type, fields)) => match fields.next() {
                    Some(field) => Some((object_type, field)),
                    None => match self.object_types.next() {
                        Some((object_type, fields)) => {
                            let mut iter = fields.iter();
                            let result = iter.next().map(|field| (object_type.as_str(), field));
                            self.fields = Some((object_type.as_str(), iter));
                            result
                        }
                        None => {
                            self.fields = None;
                            None
                        }
                    },
                },
            }
        }
    }
}
