//! Compile-time probes for expressing the syntax graph with `strutuct!`.

mod grammar {
    use m2_syn::nodes::{Adj, BinaryOperator, Expr, PostfixOperator, PrefixOperator, Symbol};
    use m2_syn::{Delimited, Punctuated, Span, Spanned};
    use these_macros_should_be_illegal::strutuct;

    strutuct! {
        #[derive(Debug, Spanned)]
        OperatorExpr
        Adjacent |AdjacentExpr| {
            left_operand: Expr,
            operator: Adj,
            right_operand: Expr,
        }
        Binary |BinaryExpr| {
            left_operand: Expr,
            operator: BinaryOperator,
            right_operand: Expr,
        }
        Prefix |PrefixExpr| {
            operator: PrefixOperator,
            operand: Expr,
        }
        Postfix |PostfixExpr| {
            operand: Expr,
            operator: PostfixOperator,
        }
    }

    strutuct! {
        #[derive(Debug, Spanned)]
        ArgumentListContent
        Symbol(Symbol)
        Pack |ArgumentPack| {
            arguments: Punctuated<Symbol>,
        }
    }

    strutuct! {
        #[derive(Debug, Spanned)]
        LambdaExpression
        arguments: LambdaArguments {
            Variadic(Symbol)
            ArgumentList(Delimited<Option<ArgumentListContent>>)
        },
        operator: m2_syn::Token![->],
        body: Expr,
    }

    #[test]
    fn constructs_a_nested_operator_specialization() {
        let left: Expr = Symbol::new("x", Span::for_text("x")).into();
        let right: Expr = Symbol::new("y", Span::for_text("y")).into();
        let adjacency = Adj::new("", Span::for_text(" "));

        let operator = OperatorExpr!(Adjacent {
            left_operand: left,
            operator: adjacency,
            right_operand: right,
        });

        let OperatorExpr::Adjacent(expression) = operator else {
            panic!("the constructor selected the wrong operator specialization");
        };
        assert_eq!(expression.left_operand.expect_span().text(), Some("x"));
        assert_eq!(expression.right_operand.expect_span().text(), Some("y"));
    }

    #[test]
    fn adds_child_derives_to_inherited_family_derives() {
        strutuct! {
            #[derive(Debug, Clone, PartialEq, Eq)]
            DerivedFamily
            #[derive(Copy, Hash)]
            kind: DerivedKind { First, Second },
        }

        fn requires_all<T: std::fmt::Debug + Clone + Copy + PartialEq + Eq + std::hash::Hash>() {}

        requires_all::<DerivedKind>();
        let value = DerivedFamily!(kind: DerivedKind!(First));
        let second = DerivedFamily!(kind: DerivedKind!(Second));
        assert_eq!(value.kind, DerivedKind::First);
        assert_eq!(second.kind, DerivedKind::Second);
    }

    #[test]
    fn keeps_a_single_argument_outside_the_punctuated_pack() {
        let single = Symbol::new("x", Span::for_text("x"));
        let arguments = LambdaArguments!(Variadic(single));

        assert!(matches!(arguments, LambdaArguments::Variadic(_)));
    }

    #[test]
    fn constructs_an_argument_pack_only_with_a_separator() {
        let arguments = Punctuated::new(
            Symbol::new("x", Span::for_text("x")),
            m2_syn::Token![,](Span::for_text(",")),
            Symbol::new("y", Span::for_text("y")),
        );
        let content = ArgumentListContent!(Pack { arguments });

        let ArgumentListContent::Pack(pack) = content else {
            panic!("the constructor selected the wrong argument-list specialization");
        };
        assert_eq!(pack.arguments.len(), 2);
    }
}

mod generated_atoms {
    use m2_syn::{Span, Spanned, syntax};

    syntax! {
        tokens {
            [=] { infix } (14, R)
            [+] { pref, bin, aug } (50, L)
            [.] [.?]
        }

        #[operator(prefix)]
        strutuct! {
            #[derive(Debug, ::m2_syn::Spanned)]
            PrefixOperator
        }

        #[operator(binary)]
        strutuct! {
            #[derive(Debug, ::m2_syn::Spanned)]
            BinaryOperator
        }

        #[operator(augmented)]
        strutuct! {
            #[derive(Debug, ::m2_syn::Spanned)]
            AugmentedOperator
        }

        #[leaf(qualified)]
        strutuct! {
            #[derive(Debug, ::m2_syn::Spanned)]
            Symbol
        }

        strutuct! {
            #[derive(Debug, ::m2_syn::Spanned)]
            MemberOwner
            operator: |MemberOperator| {
                Dot(Token![.])
                OptionalDot(Token![.?])
            },
        }
    }

    #[test]
    fn syntax_metadata_augments_explicit_atom_declarations() {
        let symbol = Symbol::new("x", Span::for_text("x"));
        assert_eq!(symbol.text(), "x");
        assert_eq!(symbol.expect_span().text(), Some("x"));

        let prefix = PrefixOperator::Add(m2_syn::Token![+](Span::for_text("+")));
        assert!(matches!(prefix, PrefixOperator::Add(_)));

        let member = MemberOperator::OptionalDot(m2_syn::Token![.?](Span::for_text(".?")));
        assert!(matches!(member, MemberOperator::OptionalDot(_)));
    }
}
