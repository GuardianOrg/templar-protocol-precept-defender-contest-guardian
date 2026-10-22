# Lending economics of the ADR-0008 bridge

How money moves in an ADR-0008 pool: who lends, who borrows, what happens when a borrower goes
underwater, and who ends up holding a loss. Written for operators and lenders, not for auditors.
Code references are to `Templar-Protocol/blend-contracts-v2` `main` at `ffeca26`.

The one-paragraph version: lenders deposit into a reserve and earn all of the interest borrowers
pay. Borrowers post collateral and borrow against it. If a borrower's collateral falls too far,
anyone can liquidate them by paying off their debt in exchange for the collateral at a discount.
If the collateral runs out before the debt is paid, the gap is taken from the lenders of that
reserve, pro rata, in the same transaction. There is no insurance fund; the "backstop" is
activation plumbing with no value. Prices come from external oracle networks through Templar's
proxy; liquidators are external firms. When prices go stale, liquidations stop but ordinary
withdrawals do not.

## 1. Reserves, lenders, borrowers

A pool has one **reserve** per asset (for example USDC, XLM, WETH). Each reserve has:

- a `supply_cap`: the most that can be deposited, in token units, sealed at setup and never
  raised afterwards (`pool/src/pool/actions.rs:302,377`; `pool/src/pool/config.rs:122-128`);
- a `c_factor` (collateral factor): how much of the asset's value counts when it is posted as
  collateral. 75% means $100 of XLM lets you borrow up to $75;
- an `l_factor` (liability factor): the inverse haircut applied to borrowed value;
- an interest curve: the borrow rate rises with utilization (how much of the reserve is lent out).

**Lenders** deposit into a reserve and receive b-tokens. The b-token price (`b_rate`) rises as
interest accrues; a lender's claim is `b_tokens * b_rate`. **Borrowers** post collateral (also
b-tokens, flagged as collateral) and take on d-tokens whose price (`d_rate`) rises with interest.

The backstop take rate is zero on this fork (`pool/src/pool/config.rs:27-29`), and there is no
protocol fee. Every unit of interest borrowers pay goes to the lenders of that reserve.

## 2. Health factor

A position is healthy while the collateral value after `c_factor` haircuts exceeds the debt
value after `l_factor` haircuts (`pool/src/pool/health_factor.rs:27-78`). The ratio is the health
factor. Below 1.0 the position is liquidatable.

Both sides are priced with the pool oracle (section 6). A position is only re-checked when its
owner acts (borrow, withdraw collateral) or when someone liquidates it. Nothing happens
automatically when a price moves; a keeper has to act.

## 3. Liquidation auctions

Only one auction type exists on this fork: the **user liquidation auction**
(`pool/src/auctions/auction.rs:85,152`). The bad-debt and interest auctions that stock Blend uses
to pay its backstop are rejected, because the backstop here has nothing to pay.

### Creating an auction

Anyone can open an auction on an underwater borrower (`new_auction`,
`pool/src/auctions/user_liquidation_auction.rs:47-52`). The creator names:

- the **bid**: which of the borrower's debts the liquidator will repay;
- the **lot**: which of the borrower's collateral the liquidator will receive;
- a **percent**: how much of the position to liquidate.

The pool checks that the resulting position would land in a reasonable health band. A partial
liquidation must leave the borrower between 1.03 and 1.15 health; above 95% of all positions it is
treated as a full liquidation (`:96-102,180-201`). This stops liquidators from taking far more
collateral than the debt justifies.

### Dutch auction pricing

An auction runs on a 400-block clock (about 35 minutes at 5-second ledgers)
(`pool/src/auctions/auction.rs:256-277`):

| Blocks since creation | Collateral the filler receives | Debt the filler repays |
| --- | --- | --- |
| 0 → 200 | 0% → 100% of the lot, rising 0.5% per block | 100% of the bid |
| 200 → 400 | 100% of the lot | 100% → 0% of the bid, falling 0.5% per block |
| 400 → deletion | 100% of the lot | **0%. The collateral is free to the first caller** (`:269-271`) |

Early on the filler pays full debt for a fraction of the collateral; later they pay a fraction of
the debt for all the collateral. Liquidators fill when the trade becomes profitable for them, which
is normally somewhere in the middle. This is the discount lenders implicitly pay for getting the
debt cleared. Past block 200 the filler repays less than the full debt; whatever they do not repay
stays with the borrower, and once the borrower has no collateral left it is written off against
the lenders (section 5). The later a fill lands, the larger that write-off.

### Filling

A filler submits `FillUserLiquidationAuction` with a `percent_filled` between 1 and 100
(`pool/src/auctions/auction.rs:201`). They take on that share of the bid (the debt) and receive
that share of the lot (the collateral). The filler's own position must be healthy at the end of
their transaction, so in practice a liquidator either has the cash to repay the debt in the same
`submit`, or holds enough of their own collateral to carry it briefly.

### Stale auctions and the free-collateral window

From block 400 until the auction is deleted, anyone can take the entire remaining lot for
nothing. The borrower's debt is not repaid; it stays on the books and lands on the lenders
(section 5). The auction can only be removed by the borrower's own `DeleteLiquidationAuction`
(allowed only while the pool is at status 0 or 1) or by anyone via `delete_stale_auction` once
500 blocks have passed (`pool/src/auctions/auction.rs:107`). Blocks 400–500 therefore have no
remedy at all.

No participant is obliged to prevent an auction reaching block 400: liquidators are external
(section 6) and borrower deletion is voluntary. Reaching it is therefore treated as a monitoring
failure on Templar's side, handled by the alerting schedule in section 6, not as something to
order a third party to prevent. "Let it expire and recreate it" is not a remedy: recreation needs
a fresh oracle price, which a stale-oracle scenario will not supply, and the lot has already been
given away.

## 4. Partial fills and liquidator capital

Partial fills mean a liquidator does not need capital equal to the whole position. The loop is:

1. fill, say, 10% of the auction: repay 10% of the debt, receive 10% of the collateral;
2. sell the collateral on the Stellar DEX for the debt asset;
3. repeat until the auction is gone.

A liquidator with $1M can clear a $10M borrower in ten rounds, provided each round fits inside the
auction clock and the collateral can be sold on the DEX without moving the price much. Those two
constraints, not the total size of the position, decide whether a liquidation succeeds:

- **Time.** Fills that land after block 400 repay nothing (section 3, "Stale auctions"). Partial
  fills shrink the existing auction; they do not restart its clock. Any lot still unfilled at 400
  is given away and its debt lands on the lenders, and a recreated auction needs a fresh oracle
  price (section 6).
- **DEX depth.** If the collateral is thin on the DEX, selling it pushes the price down and the
  next fill recovers less. This is where a liquidation "fails to sell for enough".

Practical rule: size each reserve's `supply_cap` to how much of that collateral asset can be sold
on Stellar inside roughly 30 minutes without a large price impact, and confirm with the
liquidators you expect to be active that their per-round capital times the rounds that fit in the
clock covers the largest borrower the cap allows.

## 5. When the collateral runs out: lenders take the loss

If a full fill leaves the borrower with debt but no collateral worth anything, the fork runs the
bad-debt handler in the same transaction
(`pool/src/auctions/user_liquidation_auction.rs:218-219`, `pool/src/pool/bad_debt.rs:53-108`).
Anyone can also trigger it later with `bad_debt(user)` once no auction is live.

In order:

1. **Set-off.** If the borrower also has ordinary (non-collateral) deposits in the same reserve
   they owe, those are burned against the debt first. Event: `debt_setoff`.
2. **Default to lenders.** Whatever debt remains is written off against the lenders of that
   reserve by lowering `b_rate` (`pool/src/pool/user.rs:108-113,275-286`). Every lender in the
   reserve loses the same percentage, immediately. Event: `defaulted_debt(asset, d_tokens)`.
3. **Orphan the dust.** Any collateral the borrower still holds that is worth zero in the oracle
   is moved to a pool-owned position nobody can withdraw, so a later price move cannot revive it
   for the borrower. Event: `collateral_orphaned`.

What this means for a lender:

- Your claim is `b_tokens * b_rate`. A default lowers `b_rate` for your reserve only; other
  reserves are untouched.
- The loss can never exceed what the reserve's lenders hold: the pool can only lend out cash that
  lenders deposited, so a default of the entire reserve's value is the worst case. If that happens,
  `b_rate` reaches zero, deposits and withdrawals in that reserve revert, and the reserve is dead.
  Interest from any other borrower in that reserve will slowly lift `b_rate` again; this is stock
  behaviour, not a recovery mechanism.
- There is no junior tranche, no insurance module, no protocol treasury cure. The backstop
  contract exists only so stock Blend will activate the pool. Its tokens are synthetic and have
  no value. Anyone describing it as insurance is wrong.
- Between the moment a borrower goes underwater and the moment the loss is written, lenders with
  no debt in that reserve can still withdraw at full value. The pool does not hold back cash for a
  pending loss. The slowest lenders in a bad reserve take more of the loss. This is disclosed, not
  prevented.

This is the Morpho Blue model: lenders earn the whole interest and are the only loss-absorbing
layer. It is a different model from Aave, where a staked-token safety module takes the first loss.

Why the fork does it this way rather than stock Blend's "send bad debt to the backstop": with a
worthless backstop, stock behaviour would park the bad debt in a backstop auction that no one would
ever fill. The lenders would still lose the money, but slowly and invisibly, with the last lender
to withdraw losing the most. Writing the loss immediately is the honest version of the same
outcome.

## 6. Oracle and keepers

### What the pool checks

Every price is read from the pool's oracle (Templar's proxy adapter) at the moment a position is
valued (`pool/src/pool/pool.rs:120-140`). The pool rejects a price that is:

- dated in the future;
- more than 24 hours old (exactly 24 hours is accepted);
- zero or negative;
- not 7 decimals.

The proxy itself is stricter: the protected-testnet market uses `max_age_secs = 3600`, so a price
older than one hour returns no value and the pool fails with `InvalidPrice`.

### What stops when a price is stale

A stale price only affects positions that hold that asset, but for those positions:

| Action | Blocked by a stale price? |
| --- | --- |
| Deposit, repay | No |
| Withdraw ordinary deposit | No, unless the lender also has debt in that reserve |
| Withdraw collateral, borrow | Yes, if the account has any debt |
| Create a liquidation auction | **Yes** |
| Partial fill of an existing auction | No, as long as the filler ends with no debt |
| Final fill that writes off leftover debt | **Yes** (the bad-debt check prices the position) |
| `bad_debt(user)` | Yes |
| Delete a stale auction | No |

So a stale oracle freezes liquidations and loss recognition while leaving ordinary withdrawals
open. That widens the window in section 5. Oracle liveness is therefore a lending-safety
requirement, not a convenience.

### Oracle sources and refresh

Prices come from external oracle networks (Pyth, RedStone, Chainlink) published into Templar's
proxy, which aggregates them and exposes the median through the pool adapter. Templar does not
produce prices. Two things still have to be done by somebody, and both are permissionless: calling
the proxy's `refresh` on a schedule shorter than `max_age_secs`, and extending the oracle, pool
and backstop contract TTLs so their entries are not evicted. The adapter extends its own TTL only
when it is read, so during a quiet period with no borrows, liquidations or reads nothing extends
it and the instance can be evicted while the external feeds are perfectly healthy. Templar runs
two independently funded operators for refresh and TTL upkeep while any position or auction
exists; this is a Templar duty, not an assumption about third parties.

Alerting should fire when the newest accepted price is older than half of `max_age_secs`, when
the last successful refresh is late, and when any reserve's utilization approaches 100% while an
underwater position exists (that is the run scenario in section 5).

### Liquidators

Liquidators are external, profit-seeking operators: established liquidation firms, and anyone
else who wants to. They watch health factors, open auctions on underwater borrowers, and fill them
when the lot is worth more than the bid. Templar does not run them and cannot direct their timing.
They are nonetheless the only thing that turns "lenders may lose money" into "lenders rarely lose
money": the faster an underwater position is cleared, the smaller the chance the collateral falls
below the debt before it is sold.

Their incentive and the lenders' are aligned for most of the curve in section 3 and diverge at the
end of it. For the first 200 blocks the filler repays the full debt and receives a growing
fraction of the collateral, so a fill only becomes profitable once that fraction, sold on the DEX,
exceeds the debt: late in the first 200 blocks for a position just past the liquidation threshold,
after 200 for one that has fallen further. From 200 onwards the lot is whole and the debt repaid
shrinks 0.5% per block, so every block a filler waits moves lender value to the filler. A filler
who can wait to block 399 takes all the collateral for 0.5% of the debt and hands the other 99.5%
to the lenders. What bounds the waiting is the next competitor: the first fill wins, so nobody can
afford to wait long. With only a few active firms per asset that bound is a dependency on each of
them, not a property of the market, so Templar monitors it rather than assuming it: distinct
fillers per collateral asset over a rolling window from `fill_auction` events, minimum two; below
that, the pool is frozen (status 4), which is the only immediate lever. Disabling a reserve is not
an emergency response: on a live pool the queue carries a 7-day timelock
(`pool/src/pool/config.rs:94-98`) and disabling stops new borrows while existing debt stays.

What Templar runs as a production service on its own side:

| Blocks since creation, auction unfilled | Protocol-side response |
| --- | --- |
| ~260 | Warning: the discounted bid is well below par and no filler has taken it. |
| 300 | Alert: confirm the active liquidators are live and able to see the auction. |
| 400 | The lot is now free. Record the loss event and reconcile the unrepaid debt against the reserve's `b_rate` change. |

Reserve caps sized to DEX depth (section 4) and conservative `c_factor`/`l_factor` are the
structural controls: a position liquidators can clear in a few rounds does not reach block 300,
and a wide factor buffer survives the ~35-minute auction plus fill latency.

## 7. Parameters that decide how often lenders lose

In rough order of importance:

1. `c_factor` per collateral asset sets the borrowing ceiling; the buffer against liquidation is
   the distance between a borrower's actual borrow ratio and that ceiling, not the gap between
   `c_factor` and 100%. A borrower who borrows up to the ceiling has no buffer at all, which is
   why partial liquidation targets a health factor of 1.03–1.15 rather than 1.00. What `c_factor`
   does bound is the lenders' side: once a position is liquidatable, the collateral can fall a
   further `1 - c_factor` (25% at 75%, 10% at 90%) before the debt exceeds it.
2. `supply_cap` per reserve, sized to DEX depth for that asset (section 4).
3. Proxy `max_age_secs` and the cadence of Templar's refresh operators (section 6).
4. Number of active liquidators per asset, their capital and response time; a market dependency
   Templar monitors, not a parameter it sets (sections 4 and 6).
5. The interest curve, which decides what lenders are paid for carrying this risk.

The testnet fixture values in `tools/abtest/src/deploy.rs:75-98` (stable 90%/95%, XLM 75%/75%,
WETH 80%/80%, caps effectively unlimited) are not production parameters and must be chosen
deliberately for each mainnet reserve.
