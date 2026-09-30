//! Hash a password with the same Argon2 settings the server uses, and print
//! the PHC string. Used to rotate an admin password directly in the database
//! (there is no change-password endpoint yet).
//!
//!   cargo run -p db --example hash_password -- 'new-password'

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let password = std::env::args()
        .nth(1)
        .ok_or("usage: hash_password <password>")?;
    // Round-trip through verify to guarantee the stored hash will validate.
    let hash = db::user_repo::hash_password(&password)?;
    assert!(
        db::user_repo::verify_password(&password, &hash),
        "self-check failed: hash does not verify"
    );
    println!("{hash}");
    Ok(())
}
