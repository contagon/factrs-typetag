use crate::{ImplArgs, Mode};
use proc_macro2::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse_quote, punctuated::Punctuated, ConstParam, Error, Expr, GenericParam, Ident, ItemImpl,
    Token, Type, TypeParam, TypePath,
};

pub(crate) fn expand(args: ImplArgs, mut input: ItemImpl, mode: Mode) -> TokenStream {
    // Parse name
    let name = match args.name {
        Some(name) => name.parse().unwrap(),
        None => match type_name(&input.self_ty) {
            Some(name) => name,
            None => {
                let msg = "use #[typetag::serde(name = \"...\")] to specify a unique name";
                return Error::new_spanned(&input.self_ty, msg).to_compile_error();
            }
        },
    }
    .to_token_stream();

    let name_quotes = name.to_string();
    let name_quotes = quote!(#name_quotes);

    // Add stuff to the impl
    // TODO: Need to make it work if entire name is a generic as well
    augment_impl(&mut input, &name, mode);

    let mut expanded = quote! {
        #input
    };

    let object = &input.trait_.as_ref().unwrap().1;

    if mode.de {
        // If no generics, register the type directly
        if input.generics.params.is_empty() {
            let self_ty = &input.self_ty.as_ref();
            expanded.extend(quote! {
                typetag::__private::inventory::submit! {
                    <dyn #object>::typetag_register(
                        #name_quotes,
                        (|deserializer| typetag::__private::Result::Ok(
                            typetag::__private::Box::new(
                                typetag::__private::erased_serde::deserialize::<#self_ty>(deserializer)?
                            ),
                        )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                    )
                }
            });
        }
    }

    expanded
}

fn augment_impl(input: &mut ItemImpl, name: &TokenStream, mode: Mode) {
    if mode.ser {
        // Plain name, easy peasy
        if input.generics.params.is_empty() {
            let name_quotes = name.to_string();
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    String::from(#name_quotes)
                }
            });
        } else {
            // Fill out where clauses
            input.generics.make_where_clause();
            for g in input.generics.clone().type_params() {
                let ident = &g.ident;
                input
                    .generics
                    .where_clause
                    .as_mut()
                    .unwrap()
                    .predicates
                    .push(parse_quote!(#ident: typetag::Tagged))
            }

            // If it's a blanket impl, use it's tag
            if is_blanket_impl(&input) {
                let self_ty = &input.self_ty;
                input.items.push(parse_quote! {
                    #[doc(hidden)]
                    fn typetag_name(&self) -> String {
                        <#self_ty as typetag::Tagged>::tag()
                    }
                });
            // If it's not, construct what the tag will look like
            } else {
                // Fill out extra function definitions
                let mut args = Punctuated::<Expr, Token![,]>::new();
                for g in &input.generics.params {
                    match g {
                        GenericParam::Type(TypeParam { ident, .. }) => {
                            args.push(parse_quote!(<#ident as typetag::Tagged>::tag()))
                        }
                        GenericParam::Const(ConstParam { ident, .. }) => {
                            args.push(parse_quote!(#ident))
                        }
                        GenericParam::Lifetime(_) => {}
                    }
                }

                let form = format!("{}<{}>", name, "{}".repeat(args.len()));
                input.items.push(parse_quote!(
                    #[doc(hidden)]
                    fn typetag_name(&self) -> String {
                        format!(#form, #args)
                    }
                ));
            }
        }
    }

    if mode.de {
        input.items.push(parse_quote! {
            #[doc(hidden)]
            fn typetag_deserialize(&self) {}
        });
    }
}

fn type_name(mut ty: &Type) -> Option<Ident> {
    loop {
        match ty {
            Type::Path(TypePath { qself: None, path }) => {
                return Some(path.segments.last().unwrap().ident.clone());
            }
            Type::Group(group) => {
                ty = &group.elem;
            }
            _ => return None,
        }
    }
}

fn is_blanket_impl(input: &ItemImpl) -> bool {
    let generic_names = input
        .generics
        .type_params()
        .into_iter()
        .map(|p| p.ident.clone())
        .collect::<Vec<_>>();

    // Extract name of type
    let path = match &input.self_ty.as_ref() {
        Type::Path(path) => path.path.segments.last().unwrap().ident.clone(),
        _ => return false,
    };

    generic_names.contains(&path)
}
