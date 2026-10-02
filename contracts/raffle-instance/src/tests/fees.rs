/// Tests for the protocol fee model (fix for #753).
///
/// The canonical fee model is:
///   - Fee collected **once**, at ticket purchase.
///   - Formula: `floor(total_price × protocol_fee_bp / 10000)`.
///   - Prize claims carry **no** additional fee; `platform_fee` in
///     `PrizeClaimed` is always 0.
///
/// These tests verify the total protocol take across the full buy → finalize
/// → claim lifecycle and assert it equals the documented rate.
use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, BytesN, Env, String,
};

/// Helper: create a raffle with a given protocol fee bp and ticket price,
/// run through the full lifecycle (buy → finalize → claim), and return
/// (total_paid_by_buyer, amount_received_by_winner, accumulated_fees).
fn run_full_lifecycle(
    protocol_fee_bp: u32,
    ticket_price: i128,
    num_tickets: u32,
    prize_amount: i128,
) -> (i128, i128, i128) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer = Address::generate(&env);
    let treasury = Address::generate(&env);

    let token_admin = Address::generate(&env);
    let (payment_token, token_client) = create_token(&env, &token_admin);
    // Fund creator and buyer generously.
    token_client.mint(&creator, &(prize_amount + 1_000_000_000));
    token_client.mint(&buyer, &(ticket_price * num_tickets as i128 + 1_000_000_000));

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "fee lifecycle test"),
        end_time: 0,
        no_deadline: true,
        max_tickets: num_tickets,
        max_tickets_per_tx: num_tickets,
        max_tickets_per_address: 0,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price,
        payment_token: payment_token.clone(),
        prize_amount,
        prizes: soroban_sdk::vec![&env, 10_000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp,
        treasury_address: Some(treasury.clone()),
        swap_router: None,
        tikka_token: None,
        unique_winners: false,
        metadata_hash: BytesN::from_array(&env, &[42; 32]),
        claim_lockup_seconds: Some(0),
        swap_deadline_seconds: Some(0),
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
        category: None,
        bundles: soroban_sdk::Vec::new(&env),
        prize_token: None,
        nft_contract: None,
    };

    client.init(&factory, &admin, &creator, &config);
    // Remove factory from storage so buy_tickets skips the factory call.
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&DataKey::Factory);
    });
    client.deposit_prize();

    let buyer_balance_before = token::Client::new(&env, &payment_token).balance(&buyer);
    client.buy_tickets(&buyer, &num_tickets);
    let buyer_balance_after_buy = token::Client::new(&env, &payment_token).balance(&buyer);
    let total_paid = buyer_balance_before - buyer_balance_after_buy;

    // Finalize (no oracle needed for Internal mode).
    client.finalize_raffle();

    // Advance past the claim lockup.
    env.ledger().set_timestamp(2_000);

    // The single winner is whoever holds ticket 1 (buyer).
    let raffle = client.get_raffle();
    let winner_addr = raffle.winners.get(0).unwrap().address;
    let winner_balance_before = token::Client::new(&env, &payment_token).balance(&winner_addr);
    client.claim_prize(&winner_addr, &0);
    let winner_balance_after = token::Client::new(&env, &payment_token).balance(&winner_addr);
    let amount_received = winner_balance_after - winner_balance_before;

    let accumulated_fees = client.get_accumulated_fees();

    (total_paid, amount_received, accumulated_fees)
}

/// The total protocol take (treasury receipts) across buy + claim must equal
/// exactly `floor(total_ticket_revenue × protocol_fee_bp / 10000)`.
/// No additional fee must be taken at claim time.
#[test]
fn protocol_fee_charged_only_at_purchase_floor_division() {
    // 5% fee, 100 XLM ticket, 10 tickets.
    let ticket_price = 100_000i128; // 100 XLM in base units (1e5 per XLM)
    let num_tickets = 10u32;
    let protocol_fee_bp = 500u32; // 5%
    let prize_amount = ticket_price * num_tickets as i128; // funded at gross revenue

    let (total_paid, amount_received, accumulated_fees) =
        run_full_lifecycle(protocol_fee_bp, ticket_price, num_tickets, prize_amount);

    let expected_fee = total_paid * protocol_fee_bp as i128 / 10_000;
    assert_eq!(
        accumulated_fees, expected_fee,
        "accumulated fees must equal floor(total_paid × bp / 10000)"
    );

    // Winner receives the full prize — no claim-time deduction.
    assert_eq!(
        amount_received, prize_amount,
        "winner must receive full prize amount; claim-time fee is not implemented"
    );
}

/// At the maximum allowed fee (20%), the total take is still only purchase-time.
#[test]
fn protocol_fee_at_max_bp_still_purchase_only() {
    let ticket_price = MIN_TICKET_PRICE;
    let num_tickets = 1u32;
    let protocol_fee_bp = MAX_PROTOCOL_FEE_BP; // 20%
    let prize_amount = ticket_price * 100; // generous prize

    let (_total_paid, amount_received, accumulated_fees) =
        run_full_lifecycle(protocol_fee_bp, ticket_price, num_tickets, prize_amount);

    let expected_fee = ticket_price * MAX_PROTOCOL_FEE_BP as i128 / 10_000;
    assert_eq!(accumulated_fees, expected_fee);

    // No fee at claim.
    assert_eq!(amount_received, prize_amount);
}

/// Zero fee: no treasury transfer, winner gets the full prize.
#[test]
fn zero_protocol_fee_no_treasury_transfer() {
    let ticket_price = MIN_TICKET_PRICE;
    let num_tickets = 3u32;
    let prize_amount = ticket_price * num_tickets as i128;

    let (total_paid, amount_received, accumulated_fees) =
        run_full_lifecycle(0, ticket_price, num_tickets, prize_amount);

    assert_eq!(accumulated_fees, 0, "no fees should accumulate when fee bp is 0");
    assert_eq!(total_paid, ticket_price * num_tickets as i128);
    assert_eq!(amount_received, prize_amount);
}

/// preview_buy (BuyQuote) must report a zero claim_fee and the same purchase
/// fee that buy_tickets actually charges.
#[test]
fn preview_buy_matches_actual_charge_and_shows_no_claim_fee() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000);

    let factory = env.register(MockFactory, ());
    let admin = Address::generate(&env);
    let creator = Address::generate(&env);
    let buyer = Address::generate(&env);
    let treasury = Address::generate(&env);

    let ticket_price = 200_000i128;
    let protocol_fee_bp = 250u32; // 2.5%
    let num_tickets = 5u32;
    let prize_amount = ticket_price * 100;

    let token_admin = Address::generate(&env);
    let (payment_token, token_client) = create_token(&env, &token_admin);
    token_client.mint(&creator, &(prize_amount + 1_000_000_000));
    token_client.mint(&buyer, &(ticket_price * num_tickets as i128 + 1_000_000_000));

    let contract_id = env.register(Contract, ());
    let client = ContractClient::new(&env, &contract_id);

    let config = RaffleConfig {
        description: String::from_str(&env, "preview consistency test"),
        end_time: 0,
        no_deadline: true,
        max_tickets: 100,
        max_tickets_per_tx: num_tickets,
        max_tickets_per_address: 0,
        min_tickets: 1,
        allow_multiple: true,
        ticket_price,
        payment_token: payment_token.clone(),
        prize_amount,
        prizes: soroban_sdk::vec![&env, 10_000u32],
        randomness_source: RandomnessSource::Internal,
        oracle_address: None,
        protocol_fee_bp,
        treasury_address: Some(treasury.clone()),
        swap_router: None,
        tikka_token: None,
        unique_winners: false,
        metadata_hash: BytesN::from_array(&env, &[55; 32]),
        claim_lockup_seconds: Some(0),
        swap_deadline_seconds: Some(0),
        early_bird_ticket_percentage: 0,
        early_bird_discount_bp: 0,
        category: None,
        bundles: soroban_sdk::Vec::new(&env),
        prize_token: None,
        nft_contract: None,
    };

    client.init(&factory, &admin, &creator, &config);
    env.as_contract(&contract_id, || {
        env.storage().instance().remove(&DataKey::Factory);
    });
    client.deposit_prize();

    // Get quote before buying.
    let quote = client.preview_buy(&num_tickets);
    let gross = ticket_price * num_tickets as i128;
    let expected_fee = gross * protocol_fee_bp as i128 / 10_000; // floor
    assert_eq!(quote.gross, gross);
    assert_eq!(quote.fee, expected_fee, "preview fee must use floor division");
    assert_eq!(quote.net_to_pay, gross, "no early-bird discount in this test");

    // Now actually buy and measure what was charged.
    let buyer_balance_before = token::Client::new(&env, &payment_token).balance(&buyer);
    client.buy_tickets(&buyer, &num_tickets);
    let buyer_balance_after = token::Client::new(&env, &payment_token).balance(&buyer);
    let actual_charged = buyer_balance_before - buyer_balance_after;

    assert_eq!(
        actual_charged, quote.net_to_pay,
        "buy_tickets must charge exactly what preview_buy quoted"
    );
    assert_eq!(
        client.get_accumulated_fees(),
        expected_fee,
        "accumulated fee must match the quoted floor fee"
    );
}
