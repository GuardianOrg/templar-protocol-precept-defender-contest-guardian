//! Testnet-only faucet: anyone may mint any amount of any token that has
//! appointed this contract as its admin. The owner can hand admin back.

#![no_std]

use soroban_sdk::{contract, contractimpl, token::StellarAssetClient, Address, Env};
#[cfg(test)]
mod tests;

const OWNER: &str = "owner";

#[contract]
pub struct TokenFaucet;

#[contractimpl]
impl TokenFaucet {
    pub fn __constructor(env: Env, owner: Address) {
        env.storage().instance().set(&OWNER, &owner);
    }

    pub fn owner(env: Env) -> Address {
        env.storage().instance().get(&OWNER).unwrap()
    }

    pub fn mint(env: Env, token: Address, to: Address, amount: i128) {
        StellarAssetClient::new(&env, &token).mint(&to, &amount);
    }

    pub fn set_token_admin(env: Env, token: Address, new_admin: Address) {
        Self::owner(env.clone()).require_auth();
        StellarAssetClient::new(&env, &token).set_admin(&new_admin);
    }
}
