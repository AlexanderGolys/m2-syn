//! Reads a plain Rust syntax declaration, as written in `m2_syn::nodes`, into
//! the definitions the syntax expansion works from.
//!
//! Fields are read by their Rust types. Storage is recognised by field name:
//! `delimiter: Token![()]` is the pair around a node, and `span` or a
//! symbol's `ident` is a terminal's own source.

use super::{
    Delimiter, EnumDefinition, FieldDefinition, LeafKind, Literal, Shape, StructDefinition,
    StructFields, VariantDefinition, Wrapper,
};
use crate::tokens::token_type_name;
use quote::quote;
use syn::{
    Data, DeriveInput, Error, Fields, GenericArgument, Ident, PathArguments, Result, Type,
    TypeMacro,
};

/// The definition of one derived syntax type.
pub(super) enum Definition {
    Struct(StructDefinition),
    Enum(EnumDefinition),
}

/// The options of `#[syntax(..)]`.
#[derive(Default)]
struct Options {
    leaf: Option<LeafKind>,
    literal: Option<Literal>,
    cst_kind: Option<String>,
    cell_stream: bool,
}

/// Fields a terminal stores about itself rather than about its children.
const TERMINAL_STORAGE: [&str; 2] = ["span", "ident"];

pub(super) fn read(input: &DeriveInput) -> Result<Definition> {
    let options = Options::read(input)?;
    let name = input.ident.clone();
    match &input.data {
        Data::Struct(data) => read_struct(name, &data.fields, options).map(Definition::Struct),
        Data::Enum(data) => {
            let variants = data
                .variants
                .iter()
                .map(|variant| match &variant.fields {
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => Ok(VariantDefinition {
                        name: variant.ident.clone(),
                        shape: Shape::read(&fields.unnamed[0].ty)?,
                    }),
                    _ => Err(Error::new_spanned(
                        variant,
                        "variants carry one syntax type",
                    )),
                })
                .collect::<Result<_>>()?;
            Ok(Definition::Enum(EnumDefinition {
                name,
                variants,
                operator: None,
            }))
        }
        Data::Union(_) => Err(Error::new_spanned(
            input,
            "syntax types are structs or enums",
        )),
    }
}

fn read_struct(name: Ident, fields: &Fields, options: Options) -> Result<StructDefinition> {
    let field_name = |field: &syn::Field| field.ident.clone().expect("syntax fields are named");
    let delimiter = fields
        .iter()
        .find(|field| field_name(field) == "delimiter")
        .map(|field| Delimiter::read(&field.ty))
        .transpose()?;
    let wrapper = match (delimiter, options.literal) {
        (Some(delimiter), None) => Some(Wrapper::Delimited(delimiter)),
        (None, literal) => literal.map(Wrapper::Literal),
        (Some(_), Some(_)) => {
            return Err(Error::new(name.span(), "a literal has no delimiter pair"));
        }
    };
    let fields = match options.leaf {
        Some(leaf) => StructFields::Leaf(leaf),
        None => StructFields::Product(
            fields
                .iter()
                .filter(|field| {
                    let name = field_name(field);
                    name != "delimiter" && !TERMINAL_STORAGE.iter().any(|storage| name == storage)
                })
                .map(|field| {
                    Ok(FieldDefinition {
                        name: field_name(field),
                        shape: Shape::read(&field.ty)?,
                    })
                })
                .collect::<Result<_>>()?,
        ),
    };
    Ok(StructDefinition {
        name,
        fields,
        wrapper,
        cst_kind: options.cst_kind,
        cell_stream: options.cell_stream,
    })
}

impl Options {
    fn read(input: &DeriveInput) -> Result<Self> {
        let mut options = Self::default();
        for attribute in input
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("syntax"))
        {
            attribute.parse_nested_meta(|meta| {
                let key = meta.path.require_ident()?.to_string();
                if key == "cell_stream" {
                    options.cell_stream = true;
                    return Ok(());
                }
                let value: Ident = meta.value()?.parse()?;
                let unknown = || Error::new(value.span(), format!("unknown {key} kind"));
                match (key.as_str(), value.to_string().as_str()) {
                    ("cst", kind) => options.cst_kind = Some(kind.to_owned()),
                    ("literal", "string") => options.literal = Some(Literal::String),
                    ("literal", "raw_string") => options.literal = Some(Literal::RawString),
                    ("leaf", kind) => {
                        options.leaf = Some(LeafKind::read(kind).ok_or_else(unknown)?)
                    }
                    _ => return Err(unknown()),
                }
                Ok(())
            })?;
        }
        Ok(options)
    }
}

impl LeafKind {
    fn read(kind: &str) -> Option<Self> {
        Some(match kind {
            "text" => Self::Text,
            "symbol" => Self::Symbol,
            "empty" => Self::Empty,
            "float" => Self::Float,
            "integer" => Self::Integer,
            _ => return None,
        })
    }
}

impl Delimiter {
    /// Reads `Token![()]`, `Token![[]]`, `Token![{}]`, or
    /// `Token![<||>]`.
    fn read(ty: &Type) -> Result<Self> {
        let Type::Macro(TypeMacro { mac, .. }) = ty else {
            return Err(Error::new_spanned(ty, "expected `Token![..]`"));
        };
        Self::named(&mac.tokens)
    }
}

impl Shape {
    /// Reads a field type: a syntax type, `Token![..]`, or one of `Box`,
    /// `Option`, `Vec`, `Punctuated`, and `Delimited` around another.
    fn read(ty: &Type) -> Result<Self> {
        if let Type::Macro(TypeMacro { mac, .. }) = ty
            && mac.path.is_ident("Token")
        {
            return Ok(Self::token(token_type_name(&mac.tokens)?, &mac.tokens));
        }
        let Type::Path(path) = ty else {
            return Err(Error::new_spanned(ty, "unsupported syntax field type"));
        };
        let segment = path.path.segments.last().expect("paths have a segment");
        let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
            return Ok(Self::Base(quote!(#ty), segment.ident.clone()));
        };
        let Some(GenericArgument::Type(inner)) = arguments.args.first() else {
            return Err(Error::new_spanned(ty, "expected one type argument"));
        };
        let inner = Box::new(Self::read(inner)?);
        let element = |inner: Box<Self>| match *inner {
            Self::Base(..) => Ok(inner),
            _ => Err(Error::new_spanned(
                ty,
                "repetition elements must be plain syntax types",
            )),
        };
        Ok(match segment.ident.to_string().as_str() {
            "Box" => Self::Boxed(inner),
            "Option" => Self::Optional(inner),
            "Delimited" => Self::Delimited(inner),
            "Vec" => Self::Repeated(element(inner)?),
            "Punctuated" => Self::Punctuated(element(inner)?),
            _ => return Err(Error::new_spanned(ty, "unsupported syntax wrapper")),
        })
    }
}
