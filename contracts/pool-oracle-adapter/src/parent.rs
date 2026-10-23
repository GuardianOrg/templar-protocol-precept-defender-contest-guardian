use soroban_sdk::{contractclient, Address, Env, Vec};
use templar_proxy_oracle_soroban_common::{Asset, ContractError};

#[contractclient(name = "ParentIntrospectionClient")]
pub trait ParentIntrospection {
    fn registered_assets(env: Env) -> Result<Vec<Asset>, ContractError>;
    fn get_owner(env: Env) -> Option<Address>;
}
