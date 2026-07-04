//! One-off: print Raydium CPMM static PDAs + hydrate known pools for ALT planning.
use ifx_raydium::constants::{AUTH_SEED, CPMM_PROGRAM_ID, NATIVE_MINT};
use ifx_raydium::pool::PoolRef;
use ifx_raydium::rpc::{parse_commitment, RpcPoolLoader};
use solana_sdk::pubkey::Pubkey;

fn main() {
    let (authority, _) = Pubkey::find_program_address(&[AUTH_SEED], &CPMM_PROGRAM_ID);
    let token_program = ifx_raydium::constants::TOKEN_PROGRAM_ID;
    let fee_recipient: Pubkey = "BKNnVDyzcPGCWnk8zX3Cn2KKhLASk5iTjVpxUW7YTb8P".parse().unwrap();
    let ray: Pubkey = "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R".parse().unwrap();
    let usdt: Pubkey = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".parse().unwrap();

    println!("CPMM_PROGRAM_ID={CPMM_PROGRAM_ID}");
    println!("CPMM_AUTHORITY={authority}");
    println!(
        "ASSOCIATED_TOKEN_PROGRAM={}",
        ifx_raydium::constants::ASSOCIATED_TOKEN_PROGRAM_ID
    );
    println!("fee_recipient={fee_recipient}");
    println!(
        "fee_usdt_ata={}",
        ifx_raydium::swap::user_ata(&fee_recipient, &usdt, &token_program)
    );
    println!("ray_mint={ray}");
    println!("usdt_mint={usdt}");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let rpc_url = std::env::var("SOLANA_RPC_URL").unwrap_or_else(|_| {
            "https://api.mainnet-beta.solana.com".to_string()
        });
        let rpc = RpcPoolLoader::new(&rpc_url, parse_commitment("confirmed").unwrap());

        let pairs: &[(&str, Pubkey, &str)] = &[
            (
                "SOL/RAY",
                "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iCNLY3QrkX6R".parse().unwrap(),
                "HcRKPTNnd6av9CUYAPmDZFvxFNdGQQXTxqa7ysYHG12o",
            ),
            (
                "SOL/USDC",
                "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v".parse().unwrap(),
                "",
            ),
            (
                "RAY/USDT",
                "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB".parse().unwrap(),
                "",
            ),
        ];

        for (label, other_mint, pool_hint) in pairs {
            println!("\n## {label}");
            if !pool_hint.is_empty() {
                let pool_id: Pubkey = pool_hint.parse().unwrap();
                let hint = PoolRef {
                    pool_id,
                    amm_config: None,
                    mint_a: ifx_raydium::pool::MintInfo {
                        address: NATIVE_MINT,
                        token_program: ifx_raydium::constants::TOKEN_PROGRAM_ID,
                        decimals: 9,
                        symbol: "SOL".into(),
                    },
                    mint_b: ifx_raydium::pool::MintInfo {
                        address: *other_mint,
                        token_program: ifx_raydium::constants::TOKEN_PROGRAM_ID,
                        decimals: 6,
                        symbol: String::new(),
                    },
                    trade_fee_rate: None,
                    tvl: None,
                };
                if let Ok(p) = rpc.hydrate_pool(&hint).await {
                    print_pool(&p);
                }
            }
            if let Ok(pools) = rpc.find_all_cpmm_pools(&NATIVE_MINT, other_mint).await {
                println!("rpc_cpmm_count={}", pools.len());
                for hint in pools.iter().take(3) {
                    if let Ok(p) = rpc.hydrate_pool(hint).await {
                        print_pool(&p);
                    }
                }
            }
        }
    });
}

fn print_pool(p: &ifx_raydium::pool::CpmmPool) {
    println!("pool_id={}", p.pool_id);
    println!("  amm_config={}", p.amm_config);
    println!("  authority={}", p.authority);
    println!("  observation={}", p.observation_state);
    println!("  mint0={} vault0={}", p.token_0_mint.address, p.token_0_vault);
    println!("  mint1={} vault1={}", p.token_1_mint.address, p.token_1_vault);
}
