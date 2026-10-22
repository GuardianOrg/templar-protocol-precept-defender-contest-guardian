#![allow(clippy::should_panic_without_expect)]

extern crate std;

use super::*;

use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, String,
};

mod mock_token {
    use super::*;

    soroban_sdk::contractimport!(file = "tests/fixtures/mock_sep_41_token.wasm");
}

struct Fixture {
    env: Env,
    owner: Address,
    faucet_id: Address,
    faucet: TokenFaucetClient<'static>,
    original_admin: Address,
    token_id: Address,
    token: mock_token::Client<'static>,
}

fn fixture() -> Fixture {
    let env = Env::default();
    let owner = Address::generate(&env);
    let faucet_id = env.register(TokenFaucet, (&owner,));
    let faucet = TokenFaucetClient::new(&env, &faucet_id);

    let original_admin = Address::generate(&env);
    let token_id = env.register(mock_token::WASM, ());
    let token = mock_token::Client::new(&env, &token_id);
    let name = String::from_str(&env, "Mock");
    let symbol = String::from_str(&env, "MOCK");
    token.initialize(&original_admin, &7, &name, &symbol);

    Fixture {
        env,
        owner,
        faucet_id,
        faucet,
        original_admin,
        token_id,
        token,
    }
}

fn hand_admin_to_faucet(f: &Fixture) {
    f.token
        .mock_auths(&[MockAuth {
            address: &f.original_admin,
            invoke: &MockAuthInvoke {
                contract: &f.token_id,
                fn_name: "set_admin",
                args: (&f.faucet_id,).into_val(&f.env),
                sub_invokes: &[],
            },
        }])
        .set_admin(&f.faucet_id);
}

#[test]
fn owner_returns_constructor_owner() {
    let f = fixture();
    assert_eq!(f.faucet.owner(), f.owner);
}

#[test]
#[should_panic]
fn mint_fails_while_token_admin_is_original_admin() {
    let f = fixture();
    let to = Address::generate(&f.env);
    f.faucet.mint(&f.token_id, &to, &1_000);
}

#[test]
fn handover_allows_permissionless_mint() {
    let f = fixture();
    hand_admin_to_faucet(&f);
    let to = Address::generate(&f.env);
    // No auths mocked: the faucet is now the token admin and self-authorizes.
    f.faucet.mint(&f.token_id, &to, &1_000);
    assert_eq!(f.token.balance(&to), 1_000);
}

#[test]
#[should_panic]
fn original_admin_cannot_mint_directly_after_handover() {
    let f = fixture();
    hand_admin_to_faucet(&f);
    // Original admin's auth no longer authorizes mint; admin is the faucet.
    f.token
        .mock_auths(&[MockAuth {
            address: &f.original_admin,
            invoke: &MockAuthInvoke {
                contract: &f.token_id,
                fn_name: "mint",
                args: (&f.original_admin, &100i128).into_val(&f.env),
                sub_invokes: &[],
            },
        }])
        .mint(&f.original_admin, &100);
}

#[test]
#[should_panic]
fn set_token_admin_requires_owner_auth() {
    let f = fixture();
    hand_admin_to_faucet(&f);
    f.faucet.set_token_admin(&f.token_id, &f.original_admin);
}

#[test]
fn owner_handback_restores_direct_mint() {
    let f = fixture();
    hand_admin_to_faucet(&f);
    f.faucet
        .mock_auths(&[MockAuth {
            address: &f.owner,
            invoke: &MockAuthInvoke {
                contract: &f.faucet_id,
                fn_name: "set_token_admin",
                args: (&f.token_id, &f.original_admin).into_val(&f.env),
                sub_invokes: &[],
            },
        }])
        .set_token_admin(&f.token_id, &f.original_admin);
    f.token
        .mock_auths(&[MockAuth {
            address: &f.original_admin,
            invoke: &MockAuthInvoke {
                contract: &f.token_id,
                fn_name: "mint",
                args: (&f.original_admin, &500i128).into_val(&f.env),
                sub_invokes: &[],
            },
        }])
        .mint(&f.original_admin, &500);
    assert_eq!(f.token.balance(&f.original_admin), 500);
}

#[test]
#[should_panic]
fn mint_via_faucet_fails_after_handback() {
    let f = fixture();
    hand_admin_to_faucet(&f);
    f.faucet
        .mock_auths(&[MockAuth {
            address: &f.owner,
            invoke: &MockAuthInvoke {
                contract: &f.faucet_id,
                fn_name: "set_token_admin",
                args: (&f.token_id, &f.original_admin).into_val(&f.env),
                sub_invokes: &[],
            },
        }])
        .set_token_admin(&f.token_id, &f.original_admin);
    let to = Address::generate(&f.env);
    f.faucet.mint(&f.token_id, &to, &1_000);
}
