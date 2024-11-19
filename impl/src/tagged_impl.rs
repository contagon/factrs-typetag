use crate::{ImplArgs, Mode};
use proc_macro2::{Span, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    parse_quote, punctuated::Punctuated, ConstParam, Error, Expr, GenericParam, Ident, ItemImpl,
    PathArguments, Token, Type, TypeParam, TypePath,
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

    let name_lower = name.to_string().to_lowercase();
    let name_lower = syn::Ident::new(&name_lower, Span::call_site()).to_token_stream();
    let name_quotes = name.to_string();
    let name_quotes = quote!(#name_quotes);

    // Parse generics
    // TODO: default option

    augment_impl(&mut input, &name, mode);

    let object = &input.trait_.as_ref().unwrap().1;
    let this = &input.self_ty;

    let mut expanded = quote! {
        #input
    };

    if mode.de {
        if input.generics.params.is_empty() {
            expanded.extend(quote! {
                typetag::__private::inventory::submit! {
                    <dyn #object>::typetag_register(
                        #name_quotes,
                        (|deserializer| typetag::__private::Result::Ok(
                            typetag::__private::Box::new(
                                typetag::__private::erased_serde::deserialize::<#this>(deserializer)?
                            ),
                        )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                    )
                }
            });
        } else {
            // Get name of type without generics
            let mut path = match this.as_ref() {
                Type::Path(path) => path.path.clone(),
                _ => {
                    return Error::new_spanned(&this, "Expected path for generic version")
                        .to_compile_error()
                }
            };
            path.segments.last_mut().unwrap().arguments = PathArguments::None;

            // Add in macro
            expanded.extend(quote! {
                mod register {
                    // #[allow(unused_macros)]
                    macro_rules! #name_lower {
                        ($(<$($generic:ident),*>),* $(,)?) => {$(
                            typetag::__private::inventory::submit! {
                                <dyn #object>::typetag_register(
                                    concat!(#name_quotes, "<", $(stringify!($generic)),*, ">"),
                                    (|deserializer| typetag::__private::Result::Ok(
                                        typetag::__private::Box::new(
                                            typetag::__private::erased_serde::deserialize::<#path<$($generic),*>>(deserializer)?
                                        ),
                                    )) as typetag::__private::DeserializeFn<<dyn #object as typetag::__private::Strictest>::Object>,
                                )
                            }
                        )*}
                    }
                    pub(crate) use #name_lower;
                }
            });
        }
    }

    expanded
}

fn augment_impl(input: &mut ItemImpl, name: &TokenStream, mode: Mode) {
    if mode.ser {
        if !input.generics.params.is_empty() {
            // Parse all generics
            let mut args = Punctuated::<Expr, Token![,]>::new();
            for p in &input.generics.params {
                match p {
                    GenericParam::Type(TypeParam { ident, .. }) => {
                        match &mut input.generics.where_clause {
                            Some(ref mut wc) => {
                                wc.predicates.push(parse_quote!(#ident: typetag::Tagged))
                            }
                            None => {
                                input.generics.where_clause =
                                    Some(parse_quote!(where #ident: typetag::Tagged))
                            }
                        }
                        args.push(parse_quote!(<#ident as typetag::Tagged>::tag()));
                    }
                    GenericParam::Const(ConstParam { ident, .. }) => {
                        args.push(parse_quote!(#ident));
                    }
                    GenericParam::Lifetime(_) => {}
                }
            }

            let form = format!("{}<{}>", name, "{}".repeat(args.len()));

            // TODO: Iterate over generics to make this item
            // We'll try to use it out of the box for factrs
            input.items.push(parse_quote!(
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    format!(#form, #args)
                }
            ));

        // Otherwise just handle things regularly
        } else {
            let name_quotes = name.to_string();
            input.items.push(parse_quote! {
                #[doc(hidden)]
                fn typetag_name(&self) -> String {
                    String::from(#name_quotes)
                }
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
