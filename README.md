# hylo-quote-cli

A small, read-only command-line tool that prints hyUSD quotes from the
[Hylo](https://hylo.so) protocol on Solana mainnet.

It reads Hylo's on-chain accounts over RPC and computes the quote locally with
the official [`hylo-quotes`](https://crates.io/crates/hylo-quotes) crate. It
uses no keypair and never signs or sends a transaction.

## Relationship to the official SDK

This is an unofficial tool and is not affiliated with Hylo. All quote math
comes from `hylo-quotes` 2.7.3 (pinned) from the
[hylo-so/sdk](https://github.com/hylo-so/sdk) repository; this project only
adds argument parsing and output formatting. For integrations with Jupiter, see
the official `hylo-jupiter` crate in that repository.

## Install

Requires a Rust toolchain (developed with rustc/cargo 1.98.1).

```sh
git clone <this repository>
cd hylo-quote-cli
cargo build --release --locked
# binary: target/release/hylo-quote-cli
```

## Usage

```sh
hylo-quote-cli quote --from <JitoSOL|USDC|hyUSD> --to <JitoSOL|USDC|hyUSD> --amount <number> [--json] [--retries N]
```

`--amount` is a decimal amount of the input token (for example `1.5`).
Supported pairs are the ones `hylo-quotes` supports between these three tokens,
for example JitoSOL → hyUSD, USDC → hyUSD, and hyUSD → JitoSOL/USDC.

```console
$ hylo-quote-cli quote --from USDC --to hyUSD --amount 100
In           100.000000 USDC
Out          100.000000 hyUSD
Fee          0.000000 (mint EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v)
Operation    MintStablecoinUsdc (Mint hyUSD with USDC)
Compute      100000
```

`--json` prints `in`, `out`, `fee` (amount and mint), `operation`,
`description` and `compute_units` as JSON. Amounts are decimal strings in the
token's own units, not base units. `compute_units` is the figure `hylo-quotes`
reports for the instructions it builds; those instructions are discarded.

`--retries N` (default 3, `0` disables) retries when the price oracle was
stale (exit code 3), waiting 1s, 2s, 4s, ... between attempts. Each retry
re-reads the on-chain state, and a notice is printed to stderr, so stdout (and
`--json` output) stays clean. Other errors are never retried.

### Exit codes

| Code | Meaning |
|------|---------|
| 0 | Quote printed |
| 1 | Other error (bad input, RPC failure, error returned by `hylo-quotes`) |
| 2 | hyUSD redemption not quoted because of the redeem fee curve (see below) |
| 3 | Price oracle account was still stale after all retries (`--retries 0` returns this on the first failure) |
| 4 | hyUSD → USDC amount exceeds what the USDC pool can cover (not retried) |

## RPC endpoint

The RPC URL is read from the `RPC_URL` environment variable and defaults to the
public `https://api.mainnet-beta.solana.com`, which is rate limited. Do not
commit endpoint URLs that contain API keys.

```sh
RPC_URL="https://your-rpc.example" hylo-quote-cli quote --from JitoSOL --to hyUSD --amount 1
```

## Known limitations

- **hyUSD redemption can be refused at high collateral ratio.** Hylo's
  documentation describes the redeem fee curve as defined for collateral
  ratios from 130% up to the 150% target, and states that above the domain
  "redemption is rejected (no valid fee)" and that a volatile pool "does not
  quote the redemption" if it would leave the pool's CR above the curve's upper
  bound. See [Hylo Equations](https://docs.hylo.so/technical-addendum/hylo-equations.md)
  and [Dynamic Collateral Routing](https://docs.hylo.so/protocol-overview/dynamic-collateral-routing.md).
  When this happens the tool prints an explanation and exits with code 2. The
  docs say redemptions stay available through pools below target and through the
  USDC pool, so `--to USDC` may work.
- **hyUSD → USDC is capped by the USDC pool.** The SDK refuses a hyUSD → USDC
  quote when the amount after the 0.2% redeem fee exceeds the USDC pair's
  virtual stablecoin supply (`hylo-quotes`,
  `src/token_operation/exchange.rs`, `TokenOperation<HYUSD, USDC>`), and it
  reports this as `Overflow while burning virtual stablecoin.` Despite the
  wording, it is a capacity check, not an arithmetic overflow. The tool prints
  `USDC pool cannot cover this redemption; try a smaller amount` and exits with
  code 4 without retrying. Observed on 2026-10-29: the USDC pair's virtual
  stablecoin supply was 16.590972 (equal to its vault balance), so 16.624 hyUSD
  was quoted and 16.625 hyUSD was refused. That figure is a snapshot and changes
  with the pool's state, so the cutoff will move. Hylo's docs say redemptions
  stay available through the USDC pool "while it holds enough USDC"; see
  [Dynamic Collateral Routing](https://docs.hylo.so/protocol-overview/dynamic-collateral-routing.md).
- **Quotes are computed from protocol state and are not simulated.** As the
  `hylo-quotes` documentation says, `ProtocolStateStrategy` does not check
  wallet balances.
- **Quotes move with the oracle price.** Repeated runs differ slightly.
- **Stale oracle.** Quotes that depend on the SOL/USD price can fail with
  `Oracle did not yield a price within the configured age window.` This
  occurred intermittently during testing and cleared on retry. The tool retries
  automatically (see `--retries`); if the oracle stays stale through every
  attempt it exits with code 3.
- Redeem-curve, USDC-capacity and oracle errors are recognised by matching the error text from
  `hylo-quotes`, so a wording change in a future SDK version may turn them into
  generic errors (exit code 1).
- Only JitoSOL, USDC and hyUSD are exposed. Other Hylo assets are not.

## Development

```sh
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
```

## License

MIT, see [LICENSE](LICENSE). The tool depends on the Hylo SDK crates, which are
licensed under MPL-2.0.
