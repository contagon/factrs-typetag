use proc_macro2::TokenStream;
use quote::quote;
use syn::{parse_quote, Error, ItemImpl, Type, TypePath};

use crate::{ImplArgs, Mode};

pub(crate) fn expand(args: ImplArgs, mut input: ItemImpl, mode: Mode) -> TokenStream {
    let name = match args.name {
        Some(name) => quote!(#name),
        None => match type_name(&input.self_ty) {
            Some(name) => quote!(#name),
            None => {
                let msg = "use #[typetag::serde(name = \"...\")] to specify a unique name";
                return Error::new_spanned(&input.self_ty, msg).to_compile_error();
            }
        },
    };

    augment_impl(&mut input, &name, mode);

    let object = &input.trait_.as_ref().unwrap().1;
    let this = &input.self_ty;

    let mut expanded = quote! {
        #input
    };

    if mode.de && input.generics.params.is_empty() {
        expanded.extend(quote! {
            typetag::__private::inventory::submit! {
                <dyn #object>::typetag_register(
                    #name,
                    (|deserializer| typetag::__private::Result::Ok(
                        typetag::__private::Box::new(
                            typetag::__private::erased_serde::deserialize::<#this>(deserializer)?
                        ),
                    )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                )
            }
        });
    }

    expanded
}

fn augment_impl(input: &mut ItemImpl, name: &TokenStream, mode: Mode) {
    if mode.ser {
        if input.generics.params.is_empty() {
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> &'static str {
                    #name
                }
            });
        } else {
            let self_ty = &input.self_ty;
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> &'static str {
                    <#self_ty as typetag::Tagged>::tag()
                }
            });
            input
                .generics
                .make_where_clause()
                .predicates
                .push(parse_quote! {
                    #self_ty: typetag::Tagged
                });
        }
    }

    if mode.de {
        input.items.push(parse_quote! {
            #[doc(hidden)]
            fn typetag_deserialize(&self) {}
        });
    }
}

fn type_name(mut ty: &Type) -> Option<String> {
    loop {
        match ty {
            Type::Path(TypePath { qself: None, path }) => {
                return Some(path.segments.last().unwrap().ident.to_string());
            }
            Type::Group(group) => {
                ty = &group.elem;
            }
            _ => return None,
        }
    }
}
