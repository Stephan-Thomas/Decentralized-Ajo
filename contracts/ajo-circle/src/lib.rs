#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, Map, Vec};

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum allowed fee in basis points (10 000 bp = 100%).
const MAX_FEE_BPS: u32 = 10_000;

// ── Error Types ───────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AjoError {
    NotFound = 1,
    Unauthorized = 2,
    AlreadyExists = 3,
    InvalidInput = 4,
    AlreadyPaid = 5,
    InsufficientFunds = 6,
    /// Treasury address has not been configured.
    TreasuryNotSet = 7,
    /// A checked-arithmetic operation overflowed or underflowed.
    ArithmeticError = 8,
}

// ── Data Structures ───────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CircleData {
    pub organizer: Address,
    pub token: Address,
    pub admin: Address, // Can configure platform fees
    pub contribution_amount: i128,
    pub frequency_days: u32,
    pub max_rounds: u32,
    pub current_round: u32,
    pub member_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemberData {
    pub address: Address,
    pub total_contributed: i128,
    pub total_withdrawn: i128,
    pub has_received_payout: bool,
    /// 0 = Active, 1 = Inactive, 2 = Exited
    pub status: u32,
}

/// Value returned by [`AjoCircle::claim_payout`].
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PayoutResult {
    /// Net amount that belongs to the receiving member.
    pub member_payout: i128,
    /// Platform fee to be transferred to `treasury`.
    pub fee_amount: i128,
    /// Destination address for the platform fee.
    pub treasury: Address,
}

// ── Storage Keys ──────────────────────────────────────────────────────────────

#[contracttype]
pub enum DataKey {
    Circle,
    Members,
    /// Global treasury address that collects platform fees.
    TreasuryAddress,
    /// Platform fee expressed in basis points (0–10 000).
    FeeBps,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct AjoCircle;

#[contractimpl]
impl AjoCircle {
    // ── Circle Lifecycle ──────────────────────────────────────────────────────

    /// Initialise a new Ajo circle.
    ///
    /// # Arguments
    /// * `organizer` - The address starting the circle.
    /// * `token` - The contract address of the token asset (e.g. USDC).
    /// * `admin` - Administrative address allowed to change platform fees.
    pub fn initialize_circle(
        env: Env,
        organizer: Address,
        token: Address,
        admin: Address,
        contribution_amount: i128,
        frequency_days: u32,
        max_rounds: u32,
    ) -> Result<(), AjoError> {
        organizer.require_auth();

        if contribution_amount <= 0 || frequency_days == 0 || max_rounds == 0 {
            return Err(AjoError::InvalidInput);
        }

        let circle_data = CircleData {
            organizer: organizer.clone(),
            token,
            admin,
            contribution_amount,
            frequency_days,
            max_rounds,
            current_round: 1,
            member_count: 1,
        };

        env.storage().instance().set(&DataKey::Circle, &circle_data);

        let mut members: Map<Address, MemberData> = Map::new(&env);
        members.set(
            organizer.clone(),
            MemberData {
                address: organizer,
                total_contributed: 0,
                total_withdrawn: 0,
                has_received_payout: false,
                status: 0,
            },
        );

        env.storage().instance().set(&DataKey::Members, &members);

        Ok(())
    }

    // ── Fee Configuration (admin-only) ────────────────────────────────────

    /// Configure the treasury address that will receive platform fees.
    ///
    /// Only the `admin` may call this.
    pub fn set_treasury(
        env: Env,
        caller: Address,
        treasury: Address,
    ) -> Result<(), AjoError> {
        caller.require_auth();

        let circle: CircleData = env
            .storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)?;

        if circle.admin != caller {
            return Err(AjoError::Unauthorized);
        }

        env.storage()
            .instance()
            .set(&DataKey::TreasuryAddress, &treasury);

        Ok(())
    }

    /// Set the platform fee in basis points (0 = 0 %, 10 000 = 100 %).
    ///
    /// Only the `admin` may call this.
    pub fn set_fee_bps(
        env: Env,
        caller: Address,
        fee_bps: u32,
    ) -> Result<(), AjoError> {
        caller.require_auth();

        let circle: CircleData = env
            .storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)?;

        if circle.admin != caller {
            return Err(AjoError::Unauthorized);
        }

        if fee_bps > MAX_FEE_BPS {
            return Err(AjoError::InvalidInput);
        }

        env.storage().instance().set(&DataKey::FeeBps, &fee_bps);

        Ok(())
    }

    // ── Fee Getters ───────────────────────────────────────────────────────────

    /// Return the configured treasury address, or `TreasuryNotSet` if absent.
    pub fn get_treasury(env: Env) -> Result<Address, AjoError> {
        env.storage()
            .instance()
            .get(&DataKey::TreasuryAddress)
            .ok_or(AjoError::TreasuryNotSet)
    }

    /// Return the current fee in basis points.  Defaults to **0** if never set.
    pub fn get_fee_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::FeeBps)
            .unwrap_or(0_u32)
    }

    // ── Member Management ─────────────────────────────────────────────────────

    /// Add a new member to the circle (organiser only).
    pub fn add_member(
        env: Env,
        organizer: Address,
        new_member: Address,
    ) -> Result<(), AjoError> {
        organizer.require_auth();

        let mut circle: CircleData = env
            .storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)?;

        if circle.organizer != organizer {
            return Err(AjoError::Unauthorized);
        }

        let mut members: Map<Address, MemberData> = env
            .storage()
            .instance()
            .get(&DataKey::Members)
            .ok_or(AjoError::NotFound)?;

        if members.contains_key(new_member.clone()) {
            return Err(AjoError::AlreadyExists);
        }

        members.set(
            new_member.clone(),
            MemberData {
                address: new_member,
                total_contributed: 0,
                total_withdrawn: 0,
                has_received_payout: false,
                status: 0,
            },
        );

        circle.member_count += 1;

        env.storage().instance().set(&DataKey::Members, &members);
        env.storage().instance().set(&DataKey::Circle, &circle);

        Ok(())
    }

    // ── Contributions ─────────────────────────────────────────────────────────

    /// Record a contribution from a member.
    /// This function performs a token transfer from the member to the contract.
    pub fn contribute(env: Env, member: Address, amount: i128) -> Result<(), AjoError> {
        member.require_auth();

        if amount <= 0 {
            return Err(AjoError::InvalidInput);
        }

        let circle: CircleData = env
            .storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)?;

        let mut members: Map<Address, MemberData> = env
            .storage()
            .instance()
            .get(&DataKey::Members)
            .ok_or(AjoError::NotFound)?;

        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        // Transfer funds from member to contract
        let token_client = token::Client::new(&env, &circle.token);
        token_client.transfer(&member, &env.current_contract_address(), &amount);

        member_data.total_contributed = member_data
            .total_contributed
            .checked_add(amount)
            .ok_or(AjoError::ArithmeticError)?;

        members.set(member, member_data);
        env.storage().instance().set(&DataKey::Members, &members);

        Ok(())
    }

    // ── Payout ────────────────────────────────────────────────────────────────

    /// Claim the rotating payout when it is the member's turn.
    /// 
    /// This function automatically transfers:
    /// 1. `member_payout` to the claiming member.
    /// 2. `fee_amount` to the `treasury` address.
    ///
    /// **Exact Accounting Invariant**: `member_payout + fee_amount == total_pot`
    pub fn claim_payout(env: Env, member: Address) -> Result<PayoutResult, AjoError> {
        member.require_auth();

        // Validate treasury before any computation – fail fast.
        let treasury: Address = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryAddress)
            .ok_or(AjoError::TreasuryNotSet)?;

        let circle: CircleData = env
            .storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)?;

        let mut members: Map<Address, MemberData> = env
            .storage()
            .instance()
            .get(&DataKey::Members)
            .ok_or(AjoError::NotFound)?;

        let fee_bps: u32 = env
            .storage()
            .instance()
            .get(&DataKey::FeeBps)
            .unwrap_or(0_u32);

        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        if member_data.has_received_payout {
            return Err(AjoError::AlreadyPaid);
        }

        // ── Safe arithmetic ───────────────────────────────────────────────────

        let total_payout = (circle.member_count as i128)
            .checked_mul(circle.contribution_amount)
            .ok_or(AjoError::ArithmeticError)?;

        let fee_amount = Self::compute_fee(total_payout, fee_bps)?;

        let member_payout = total_payout
            .checked_sub(fee_amount)
            .ok_or(AjoError::ArithmeticError)?;

        // ── State mutation ────────────────────────────────────────────────────

        member_data.has_received_payout = true;
        member_data.total_withdrawn = member_data
            .total_withdrawn
            .checked_add(member_payout)
            .ok_or(AjoError::ArithmeticError)?;

        members.set(member.clone(), member_data);
        env.storage().instance().set(&DataKey::Members, &members);

        // ── Transfers ─────────────────────────────────────────────────────────

        let token_client = token::Client::new(&env, &circle.token);
        
        // Final payout to the member
        if member_payout > 0 {
            token_client.transfer(&env.current_contract_address(), &member, &member_payout);
        }
        
        // Fee to treasury
        if fee_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &fee_amount);
        }

        Ok(PayoutResult {
            member_payout,
            fee_amount,
            treasury,
        })
    }

    // ── Partial Withdrawal ────────────────────────────────────────────────────

    /// Perform a partial withdrawal with a 10 % early-exit penalty.
    /// The penalty is sent to the treasury.
    pub fn partial_withdraw(env: Env, member: Address, amount: i128) -> Result<i128, AjoError> {
        member.require_auth();

        if amount <= 0 {
            return Err(AjoError::InvalidInput);
        }

        let circle: CircleData = env
            .storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)?;

        let mut members: Map<Address, MemberData> = env
            .storage()
            .instance()
            .get(&DataKey::Members)
            .ok_or(AjoError::NotFound)?;

        let mut member_data = members.get(member.clone()).ok_or(AjoError::NotFound)?;

        let available = member_data
            .total_contributed
            .checked_sub(member_data.total_withdrawn)
            .ok_or(AjoError::ArithmeticError)?;

        if amount > available {
            return Err(AjoError::InsufficientFunds);
        }

        // 10 % early-exit penalty
        let penalty = (amount * 10) / 100;
        let net_amount = amount
            .checked_sub(penalty)
            .ok_or(AjoError::ArithmeticError)?;

        member_data.total_withdrawn = member_data
            .total_withdrawn
            .checked_add(amount)
            .ok_or(AjoError::ArithmeticError)?;

        members.set(member.clone(), member_data);
        env.storage().instance().set(&DataKey::Members, &members);

        // ── Transfers ─────────────────────────────────────────────────────────
        let token_client = token::Client::new(&env, &circle.token);
        
        // Payout to member
        token_client.transfer(&env.current_contract_address(), &member, &net_amount);
        
        // Penalty to treasury (if set, otherwise stays in contract)
        if let Some(treasury) = env.storage().instance().get::<_, Address>(&DataKey::TreasuryAddress) {
            if penalty > 0 {
                token_client.transfer(&env.current_contract_address(), &treasury, &penalty);
            }
        }

        Ok(net_amount)
    }

    // ── Read-only Queries ─────────────────────────────────────────────────────

    pub fn get_circle_state(env: Env) -> Result<CircleData, AjoError> {
        env.storage()
            .instance()
            .get(&DataKey::Circle)
            .ok_or(AjoError::NotFound)
    }

    pub fn get_member_balance(env: Env, member: Address) -> Result<MemberData, AjoError> {
        let members: Map<Address, MemberData> = env
            .storage()
            .instance()
            .get(&DataKey::Members)
            .ok_or(AjoError::NotFound)?;

        members.get(member).ok_or(AjoError::NotFound)
    }

    pub fn get_members(env: Env) -> Result<Vec<MemberData>, AjoError> {
        let members: Map<Address, MemberData> = env
            .storage()
            .instance()
            .get(&DataKey::Members)
            .ok_or(AjoError::NotFound)?;

        let mut out = Vec::new(&env);
        for (_, m) in members.iter() {
            out.push_back(m);
        }

        Ok(out)
    }

    // ── Private Helpers ───────────────────────────────────────────────────────

    fn compute_fee(total_payout: i128, fee_bps: u32) -> Result<i128, AjoError> {
        if fee_bps == 0 {
            return Ok(0);
        }
        let fee = total_payout
            .checked_mul(fee_bps as i128)
            .ok_or(AjoError::ArithmeticError)?
            / 10_000_i128;
        Ok(fee)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};
    
    // Explicitly import token testutils
    use soroban_sdk::token::{StellarAssetClient, Client as TokenClient};

    struct TestEnv {
        env: Env,
        contract_id: Address,
        admin: Address,
        token_id: Address,
    }

    impl TestEnv {
        fn new() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            let contract_id = env.register_contract(None, AjoCircle);
            let admin = Address::generate(&env);
            
            // Create a mock token
            let token_admin = Address::generate(&env);
            let token_id = env.register_stellar_asset_contract(token_admin);
            
            Self { env, contract_id, admin, token_id }
        }

        fn client(&self) -> AjoCircleClient {
            AjoCircleClient::new(&self.env, &self.contract_id)
        }
        
        fn token_client(&self) -> TokenClient {
            TokenClient::new(&self.env, &self.token_id)
        }
        
        fn token_admin_client(&self) -> StellarAssetClient {
            StellarAssetClient::new(&self.env, &self.token_id)
        }
    }

    fn bootstrap(
        t: &TestEnv,
        contribution: i128,
        extra: u32,
    ) -> (Address, soroban_sdk::Vec<Address>) {
        let organizer = Address::generate(&t.env);
        
        // Mint tokens to organizer to cover initial contribution capability
        // Note: For Ajo, each round's pool is built from contributions.
        
        t.client()
            .initialize_circle(&organizer, &t.token_id, &t.admin, &contribution, &30_u32, &10_u32);

        let mut extras = soroban_sdk::Vec::new(&t.env);
        for _ in 0..extra {
            let m = Address::generate(&t.env);
            t.client().add_member(&organizer, &m);
            extras.push_back(m);
        }
        (organizer, extras)
    }

    #[test]
    fn test_initialize_circle() {
        let t = TestEnv::new();
        let organizer = Address::generate(&t.env);
        t.client()
            .initialize_circle(&organizer, &t.token_id, &t.admin, &1_000_000_i128, &30_u32, &5_u32);

        let state = t.client().get_circle_state();
        assert_eq!(state.organizer, organizer);
        assert_eq!(state.token, t.token_id);
        assert_eq!(state.admin, t.admin);
    }

    #[test]
    fn test_contribution_with_token_transfer() {
        let t = TestEnv::new();
        let (organizer, _) = bootstrap(&t, 1_000, 0);
        
        // Mint tokens to organizer
        t.token_admin_client().mint(&organizer, &1_000);
        
        t.client().contribute(&organizer, &1_000);
        
        // Check local state
        let bal = t.client().get_member_balance(&organizer);
        assert_eq!(bal.total_contributed, 1_000);
        
        // Check token balance (organizer should have 0, contract should have 1000)
        assert_eq!(t.token_client().balance(&organizer), 0);
        assert_eq!(t.token_client().balance(&t.contract_id), 1_000);
    }

    #[test]
    fn test_claim_payout_with_fee_transfer() {
        let t = TestEnv::new();
        let (organizer, extras) = bootstrap(&t, 1_000, 1); // 2 members total
        let member2 = extras.get(0).unwrap();
        let treasury = Address::generate(&t.env);

        // Set global platform fee (only admin can do this)
        t.client().set_treasury(&t.admin, &treasury);
        t.client().set_fee_bps(&t.admin, &500_u32); // 5 %

        // Mint and contribute (total pot 2000)
        t.token_admin_client().mint(&organizer, &1_000);
        t.token_admin_client().mint(&member2, &1_000);
        t.client().contribute(&organizer, &1_000);
        t.client().contribute(&member2, &1_000);
        
        assert_eq!(t.token_client().balance(&t.contract_id), 2_000);

        // Claim payout for organizer
        // 5% of 2000 = 100 fee. 1900 to member.
        let result = t.client().claim_payout(&organizer);

        assert_eq!(result.fee_amount, 100);
        assert_eq!(result.member_payout, 1_900);
        
        // Check token balances
        assert_eq!(t.token_client().balance(&organizer), 1_900);
        assert_eq!(t.token_client().balance(&treasury), 100);
        assert_eq!(t.token_client().balance(&t.contract_id), 0); // No dust!
    }

    #[test]
    fn test_partial_withdraw_with_penalty() {
        let t = TestEnv::new();
        let (organizer, _) = bootstrap(&t, 2_000, 0);
        let treasury = Address::generate(&t.env);
        t.client().set_treasury(&t.admin, &treasury);

        t.token_admin_client().mint(&organizer, &2_000);
        t.client().contribute(&organizer, &2_000);

        // Withdraw 1000. Penalty 10% = 100. Net 900.
        let net = t.client().partial_withdraw(&organizer, &1_000);
        
        assert_eq!(net, 900);
        assert_eq!(t.token_client().balance(&organizer), 900);
        assert_eq!(t.token_client().balance(&treasury), 100);
        assert_eq!(t.token_client().balance(&t.contract_id), 1_000); // Remaining funds
    }

    #[test]
    fn test_unauthorized_fee_update_fails() {
        let t = TestEnv::new();
        let (organizer, _) = bootstrap(&t, 1_000, 0);
        let fake_treasury = Address::generate(&t.env);

        // Organizer (circle owner) tries to set platform fee - FAIL
        let res = t.client().try_set_treasury(&organizer, &fake_treasury);
        assert!(res.is_err());
    }
}
