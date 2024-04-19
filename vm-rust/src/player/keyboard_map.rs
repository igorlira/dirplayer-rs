use std::{collections::HashMap, iter::FromIterator};

use lazy_static::lazy_static;

lazy_static! {
  pub static ref KEYBOARD_KEY_MAP_JS_TO_SW: HashMap<u16, u16> = HashMap::from_iter([
    (8, 51), // backspace
    (13, 36), // enter
    (32, 49), // space
    (65, 0), // a
    (69, 14), // e
    (68, 2), // d
    (84, 17), // t
    (83, 1), // s
    (97, 83), // numpad 1
    (98, 84), // numpad 2
    (99, 85), // numpad 3
    (100, 86), // numpad 4
    (101, 87), // numpad 5
    (102, 88), // numpad 6
    (103, 89), // numpad 7
    (104, 91), // numpad 8
    (105, 92), // numpad 9
  ]);
}
