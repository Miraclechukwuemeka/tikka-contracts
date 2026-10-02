# Tikka Protocol Fee Model

## Fee Collection Point

Protocol fees are collected **at ticket purchase only**. Prize claims do not
incur any additional protocol fee.

### At Ticket Purchase
- **Formula:** `floor(total_price × protocol_fee_bp / 10000)`
  where `total_price = ticket_price × quantity`.
- **Rounding Rule:** Floor (integer) division — any sub-unit remainder stays
  with the contract and is credited to the prize pool or fee accumulator. The
  protocol never rounds up against the buyer.
- **Recipient:** Treasury address (`RaffleConfig::treasury_address`). If no
  treasury is configured the fee is retained in the contract's
  `AccumulatedFees` balance and swept by the `withdraw_fees` admin function.
- **Payer:** Ticket buyer. The buyer transfers `total_price` and the contract
  internally routes `protocol_fee` to the treasury.

**Example:** 2.5% fee (`protocol_fee_bp = 250`) on a 100 XLM ticket, 1 ticket:

```
total_price  = 100 XLM
protocol_fee = floor(100 × 250 / 10000) = floor(2.5) = 2 XLM  (dust stays in contract)
net_to_raffle = 98 XLM
```

For 10 tickets at the same price:
```
total_price  = 1000 XLM
protocol_fee = floor(1000 × 250 / 10000) = 25 XLM
net_to_raffle = 975 XLM
```

### At Prize Claim

**No fee is charged.** The full prize tier amount is transferred to the winner.
The `platform_fee` field in the `PrizeClaimed` event is always `0`.

## Effective Total Fee

For a raffle with `protocol_fee_bp = 250` (2.5%), `ticket_price = 100 XLM`,
and 10 tickets:

- Ticket fees: `floor(1000 × 250 / 10000)` = 25 XLM
- Prize claim fee: 0 XLM
- **Total protocol revenue: 25 XLM (2.5% of gross ticket sales)**

The maximum fee is capped by `MAX_PROTOCOL_FEE_BP = 2000` (20%). Even at
maximum fee with 10 tickets at 100 XLM, the winner receives 100% of the prize
amount and 80% of gross ticket sales fund the prize pool.

## Tier Prize Allocation

Prize tiers are divided from `prize_amount` using floor division for all tiers
except the final one:

- **Formula:** `floor(prize_amount × tier_basis_points / 10000)` for every tier
  except the last.
- **Final tier:** Receives `prize_amount` minus the sum of all earlier tiers.
  This guarantees all tier prizes sum exactly to `prize_amount` with no dust
  left undistributed.

## Code Locations

| Component | File | Formula |
|---|---|---|
| Fee deduction | `contracts/raffle-instance/src/tickets.rs` | `floor(total_price × bp / 10000)` |
| Preview quote | `contracts/raffle-instance/src/views.rs` → `helpers.rs` | same formula |
| Prize claim | `contracts/raffle-instance/src/claim.rs` | no fee |
| Fee cap | `contracts/raffle-shared/src/constants.rs` | `MAX_PROTOCOL_FEE_BP = 2000` |
