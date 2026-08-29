use super::eval;

#[test]
fn local_load_and_store() {
    assert_eq!(eval("(let [x 19] x)"), "19");
    assert_eq!(eval("(let [x 19 y 23] (+ x y))"), "42");
    assert_eq!(eval("(let [x 1] (let [y 2] (+ x y)))"), "3");
}

#[test]
fn sequential_let_bindings_observe_earlier_names() {
    assert_eq!(eval("(let [x 19 y (+ x 23)] y)"), "42");
    assert_eq!(eval("(let [x 1 x (+ x 1)] x)"), "2");
}

#[test]
fn lexical_shadowing() {
    assert_eq!(eval("(let [x 1] (let [x 2] x))"), "2");
    assert_eq!(eval("(let [x 1] (do (let [x 2] x) x))"), "1");
    assert_eq!(eval("(let [x 1 y 2] (+ (let [x 10] x) y))"), "12");
}

#[test]
fn destructuring_executes_in_let_and_loop_bindings() {
    assert_eq!(eval("(let [[a b] [1 2]] (+ a b))"), "3");
    assert_eq!(
        eval("(loop [[a b] [1 2]] (if (< a 3) (recur [(+ a 1) (+ b 1)]) (+ a b)))"),
        "7"
    );
}
