use super::*;
use crate::struct_registry::StructKey;

#[test]
fn display_prim_int() {
    assert_eq!(Ty::int().display(), "Int");
}

#[test]
fn display_prim_float() {
    assert_eq!(Ty::float().display(), "Float");
}

#[test]
fn display_prim_text() {
    assert_eq!(Ty::text().display(), "Text");
}

#[test]
fn display_prim_rational() {
    assert_eq!(Ty::rational().display(), "Rational");
}

#[test]
fn display_unit() {
    assert_eq!(Ty::Unit.display(), "Unit");
}

#[test]
fn display_struct() {
    assert_eq!(
        Ty::Struct(StructKey::Plain("Pessoa".into())).display(),
        "Pessoa"
    );
}

#[test]
fn display_sum() {
    assert_eq!(Ty::Sum("Boolean".into()).display(), "Boolean");
}

#[test]
fn display_interface() {
    assert_eq!(Ty::Interface("NUM".into()).display(), "NUM");
}

#[test]
fn display_var() {
    assert_eq!(Ty::Var("T".into()).display(), "T");
}

#[test]
fn display_generic_single_param() {
    assert_eq!(
        Ty::Generic("Optional".into(), vec![Ty::int()]).display(),
        "Optional::Int"
    );
}

#[test]
fn display_generic_multi_param() {
    assert_eq!(
        Ty::Generic("Result".into(), vec![Ty::int(), Ty::text()]).display(),
        "Result::(Int, Text)"
    );
}

#[test]
fn display_function() {
    assert_eq!(
        Ty::Function(vec![Ty::int(), Ty::int()], Box::new(Ty::int())).display(),
        "Lambda(Int Int -> Int)"
    );
}

#[test]
fn display_function_no_params() {
    assert_eq!(
        Ty::Function(vec![], Box::new(Ty::int())).display(),
        "Lambda(-> Int)"
    );
}

#[test]
fn display_action() {
    assert_eq!(
        Ty::Action(vec![Ty::int()], Box::new(Ty::Unit)).display(),
        "Action(Int) -> Unit"
    );
}

#[test]
fn display_action_multi_param() {
    assert_eq!(
        Ty::Action(vec![Ty::int(), Ty::text()], Box::new(Ty::int())).display(),
        "Action(Int, Text) -> Int"
    );
}

#[test]
fn display_tuple() {
    assert_eq!(
        Ty::Tuple(vec![Ty::int(), Ty::text()]).display(),
        "(Int, Text)"
    );
}

#[test]
fn display_list() {
    assert_eq!(Ty::List(Box::new(Ty::int())).display(), "[Int]");
}

#[test]
fn display_array() {
    assert_eq!(Ty::Array(Box::new(Ty::int())).display(), "{Int}");
}

#[test]
fn display_dict() {
    assert_eq!(
        Ty::Dict(Box::new(Ty::text()), Box::new(Ty::int())).display(),
        "Dict::(Text, Int)"
    );
}

#[test]
fn display_set() {
    assert_eq!(Ty::Set(Box::new(Ty::int())).display(), "Set::Int");
}

#[test]
fn display_sender() {
    assert_eq!(Ty::Sender(Box::new(Ty::int())).display(), "Sender::Int");
}

#[test]
fn display_receiver() {
    assert_eq!(Ty::Receiver(Box::new(Ty::int())).display(), "Receiver::Int");
}

#[test]
fn display_receiver_factory() {
    assert_eq!(
        Ty::ReceiverFactory(Box::new(Ty::int())).display(),
        "ReceiverFactory::Int"
    );
}

#[test]
fn display_byte() {
    assert_eq!(Ty::Byte.display(), "Byte");
}

#[test]
fn display_bytes() {
    assert_eq!(Ty::Bytes.display(), "Bytes");
}

#[test]
fn display_file() {
    assert_eq!(Ty::File.display(), "File");
}

#[test]
fn display_socket() {
    assert_eq!(Ty::Socket.display(), "Socket");
}

#[test]
fn display_nested_generic() {
    assert_eq!(
        Ty::Generic("Optional".into(), vec![Ty::List(Box::new(Ty::int()))]).display(),
        "Optional::[Int]"
    );
}
