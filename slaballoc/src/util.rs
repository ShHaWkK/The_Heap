//! Utilitaires bas-niveau utilisés par l’allocator par slabs
//!
//! - `align_up`: aligne une adresse/taille vers le multiple suivant d’une puissance de deux.
//! - `is_power_of_two`: teste si une valeur est une puissance de deux strictement positive.
//!
//! Ces fonctions sont `const` et ne dépendent pas de l’environnement d’exécution.
//! Elles sont conçues pour un usage en `no_std`.
#![allow(dead_code)]

/// Aligne `x` vers le haut au multiple de `align`.
///
/// - Préconditions: `align` doit être une puissance de deux strictement positive.
/// - Retour: la plus petite valeur `>= x` telle que `result % align == 0`.
/// - Formule: `(x + align - 1) & !(align - 1)`
pub const fn align_up(x: usize, align: usize) -> usize {
    (x + align - 1) & !(align - 1)
}

/// Retourne `true` si `x` est une puissance de deux strictement positive.
///
/// - `1, 2, 4, 8, ...` -> `true`
/// - `0` ou valeurs non puissances de deux -> `false`
pub const fn is_power_of_two(x: usize) -> bool {
    x != 0 && (x & (x - 1)) == 0
}
