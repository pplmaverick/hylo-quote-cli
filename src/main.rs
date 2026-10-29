//! Read-only hyUSD quote CLI. Uses no keypair: it never signs or sends a
//! transaction. The quote's `user` is a throwaway pubkey that only shows up
//! in the (discarded) instructions.
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand, ValueEnum};
use hylo_quotes::prelude::*;
use serde_json::json;
use solana_rpc_client::nonblocking::rpc_client::RpcClient;

const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";
/// Slippage only affects the (discarded) instructions, not the quoted amounts.
const SLIPPAGE_BPS: u64 = 50;

#[derive(Parser)]
#[command(version, about = "Read-only hyUSD quotes from Hylo on Solana mainnet")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Quote a swap between hyUSD and JitoSOL/USDC. RPC endpoint is read from
    /// the RPC_URL environment variable.
    Quote {
        #[arg(long, value_enum)]
        from: Symbol,
        #[arg(long, value_enum)]
        to: Symbol,
        /// Human-readable amount of the input token, e.g. 1.5
        #[arg(long)]
        amount: String,
        /// Print machine-readable JSON instead of a table
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Symbol {
    #[value(name = "JitoSOL", alias = "jitosol")]
    JitoSol,
    #[value(name = "USDC", alias = "usdc")]
    Usdc,
    #[value(name = "hyUSD", alias = "hyusd")]
    HyUsd,
}

impl Symbol {
    fn name(self) -> &'static str {
        match self {
            Symbol::JitoSol => "JitoSOL",
            Symbol::Usdc => "USDC",
            Symbol::HyUsd => "hyUSD",
        }
    }
    fn mint(self) -> Pubkey {
        match self {
            Symbol::JitoSol => JITOSOL::MINT,
            Symbol::Usdc => USDC::MINT,
            Symbol::HyUsd => HYUSD::MINT,
        }
    }
    fn decimals(self) -> u32 {
        match self {
            Symbol::JitoSol => 9,
            Symbol::Usdc | Symbol::HyUsd => 6,
        }
    }
}

/// Parses "1.25" into base units without going through floats.
fn parse_amount(s: &str, decimals: u32) -> anyhow::Result<u64> {
    let (whole, frac) = s.split_once('.').unwrap_or((s, ""));
    let valid = |p: &str| p.chars().all(|c| c.is_ascii_digit());
    if !valid(whole) || !valid(frac) || (whole.is_empty() && frac.is_empty()) {
        anyhow::bail!("invalid amount '{s}': use a plain decimal number like 1.5");
    }
    if frac.len() > decimals as usize {
        anyhow::bail!("amount '{s}' has more than {decimals} decimal places");
    }
    let digits = format!("{whole}{frac:0<width$}", width = decimals as usize);
    let n: u64 = digits.trim_start_matches('0').parse().unwrap_or(0);
    if n == 0 {
        anyhow::bail!("amount must be greater than zero");
    }
    Ok(n)
}

/// Renders `bits * 10^exp` as an exact decimal string (exp is <= 0 here).
fn fmt_fixed(bits: u64, exp: i8) -> String {
    let scale = u32::try_from(-i32::from(exp)).unwrap_or(0);
    if scale == 0 {
        return bits.to_string();
    }
    let div = 10u64.pow(scale);
    format!(
        "{}.{:0width$}",
        bits / div,
        bits % div,
        width = scale as usize
    )
}

fn is_redeem_curve_error(msg: &str) -> bool {
    msg.contains("redeem fee") && msg.contains("curve domain")
}

#[tokio::main]
async fn main() -> ExitCode {
    match run(Cli::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, msg)) => {
            eprintln!("{msg}");
            ExitCode::from(code)
        }
    }
}

async fn run(cli: Cli) -> Result<(), (u8, String)> {
    let Command::Quote {
        from,
        to,
        amount,
        json,
    } = cli.command;
    if from == to {
        return Err((1, "error: --from and --to must differ".into()));
    }
    let amount_in =
        parse_amount(&amount, from.decimals()).map_err(|e| (1, format!("error: {e}")))?;
    let rpc_url = std::env::var("RPC_URL").unwrap_or_else(|_| DEFAULT_RPC.into());
    let rpc = Arc::new(RpcClient::new_with_commitment(
        rpc_url,
        CommitmentConfig::confirmed(),
    ));
    let strategy = ProtocolStateStrategy::new(Arc::new(RpcStateProvider::new(rpc)));
    let user = Pubkey::new_unique();

    let (q, meta) = strategy
        .runtime_quote_with_metadata(from.mint(), to.mint(), amount_in, user, SLIPPAGE_BPS)
        .await
        .map_err(|e| {
            let msg = format!("{e:#}");
            if from == Symbol::HyUsd && is_redeem_curve_error(&msg) {
                (
                    2,
                    format!(
                        "redeem not quoted: {msg}\n\
             By design (docs.hylo.so, Hylo Equations): a volatile pool does not quote a \
             redemption that would leave its collateral ratio above the redeem fee curve \
             (150%). Redemptions stay available through pools below target and through the \
             USDC pool; try `--to USDC`."
                    ),
                )
            } else if msg.contains("Oracle did not yield a price") {
                (
                    3,
                    format!(
                        "error: {msg}\n\
             The on-chain price oracle account was stale when read. This is usually \
             transient; retry in a few seconds."
                    ),
                )
            } else {
                (1, format!("error: {msg}"))
            }
        })?;

    let out = fmt_fixed(q.amount_out.bits, q.amount_out.exp);
    let inp = fmt_fixed(q.amount_in.bits, q.amount_in.exp);
    let fee = fmt_fixed(q.fee_amount.bits, q.fee_amount.exp);
    let op = format!("{:?}", meta.operation);
    if json {
        let v = json!({
          "from": from.name(), "to": to.name(),
          "in": inp, "out": out,
          "fee": { "amount": fee, "mint": q.fee_mint.to_string() },
          "operation": op, "description": meta.description,
          "compute_units": q.compute_units,
        });
        println!("{}", serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        println!("{:<12} {} {}", "In", inp, from.name());
        println!("{:<12} {} {}", "Out", out, to.name());
        println!("{:<12} {} (mint {})", "Fee", fee, q.fee_mint);
        println!("{:<12} {} ({})", "Operation", op, meta.description);
        println!("{:<12} {}", "Compute", q.compute_units);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_amounts_exactly() {
        assert_eq!(parse_amount("1", 9).ok(), Some(1_000_000_000));
        assert_eq!(parse_amount("1.5", 6).ok(), Some(1_500_000));
        assert_eq!(parse_amount("0.000001", 6).ok(), Some(1));
        assert_eq!(parse_amount(".5", 6).ok(), Some(500_000));
    }

    #[test]
    fn rejects_bad_amounts() {
        for bad in ["", ".", "abc", "-1", "0", "0.0", "1.0000001", "1e3"] {
            assert!(parse_amount(bad, 6).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn formats_fixed_point() {
        assert_eq!(fmt_fixed(129_813_136, -6), "129.813136");
        assert_eq!(fmt_fixed(7_370_000, -9), "0.007370000");
        assert_eq!(fmt_fixed(5, 0), "5");
    }

    #[test]
    fn detects_redeem_curve_error() {
        assert!(is_redeem_curve_error(
            "No valid redeem fee: projected CR above redeem fee curve domain."
        ));
        assert!(!is_redeem_curve_error(
            "No valid mint fee: projected CR below mint fee curve domain."
        ));
    }
}
