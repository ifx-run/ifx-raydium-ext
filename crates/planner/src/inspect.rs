//! v0 transaction inspection for build output (pumpfun-compatible JSON shape).

use ifx_sdk::decode_ix::ifx_ix_hint;
use serde::Serialize;
use solana_message::v0::{LoadedAddresses, LoadedMessage, MessageAddressTableLookup};
use solana_message::{AddressLookupTableAccount, MessageHeader, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::transaction::VersionedTransaction;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxAccountInspection {
    pub index: usize,
    pub pubkey: String,
    pub is_signer: bool,
    pub is_writable: bool,
    pub alt_loaded: bool,
    pub resolution: String,
    pub in_alt_table_unused: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxInstructionInspection {
    pub index: usize,
    pub program_id: String,
    pub program_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    pub accounts: Vec<TxAccountInspection>,
    pub data_hex: String,
    pub data_base64: String,
    pub data_length: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionConfigInspection {
    pub compute_unit_limit: u32,
    pub loaded_accounts_data_size_limit: u32,
    pub priority_fee_lamports: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxInspection {
    pub version: u8,
    pub num_instructions: usize,
    pub static_account_keys: usize,
    pub loaded_writable_accounts: usize,
    pub loaded_readonly_accounts: usize,
    pub total_account_keys: usize,
    pub address_lookup_tables: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_used: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fee_payer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smart_close_applied: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_size_bytes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_config: Option<TransactionConfigInspection>,
    pub instructions: Vec<TxInstructionInspection>,
}

#[derive(Debug, Clone)]
pub struct InspectOpts {
    pub ifx_program_id: Pubkey,
    pub frame_used: Option<Pubkey>,
    pub fee_payer: Option<Pubkey>,
    pub smart_close_applied: Option<bool>,
    pub transaction_size_bytes: Option<usize>,
    pub address_lookup_table_addresses: Vec<String>,
    pub transaction_config: Option<TransactionConfigInspection>,
}

/// Inspect a compiled v1 instruction list (all accounts inline; no ALTs).
pub fn inspect_instructions(
    instructions: &[solana_sdk::instruction::Instruction],
    opts: &InspectOpts,
) -> TxInspection {
    let static_keys = collect_static_keys(opts.fee_payer.as_ref(), instructions);
    let index_by_key: std::collections::HashMap<String, usize> = static_keys
        .iter()
        .enumerate()
        .map(|(i, pk)| (pk.clone(), i))
        .collect();

    let inspected: Vec<TxInstructionInspection> = instructions
        .iter()
        .enumerate()
        .map(|(index, ix)| {
            let program_id = ix.program_id.to_string();
            let data = &ix.data;
            let accounts = ix
                .accounts
                .iter()
                .map(|k| {
                    let pubkey = k.pubkey.to_string();
                    TxAccountInspection {
                        index: *index_by_key.get(&pubkey).unwrap_or(&usize::MAX),
                        pubkey,
                        is_signer: k.is_signer,
                        is_writable: k.is_writable,
                        alt_loaded: false,
                        resolution: "static".into(),
                        in_alt_table_unused: false,
                    }
                })
                .collect();

            TxInstructionInspection {
                index,
                program_id: program_id.clone(),
                program_label: program_label(&program_id, &opts.ifx_program_id),
                hint: instruction_hint(&program_id, data, &opts.ifx_program_id),
                accounts,
                data_hex: bytes_to_hex(data),
                data_base64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    data,
                ),
                data_length: data.len(),
            }
        })
        .collect();

    TxInspection {
        version: 1,
        num_instructions: inspected.len(),
        static_account_keys: static_keys.len(),
        loaded_writable_accounts: 0,
        loaded_readonly_accounts: 0,
        total_account_keys: static_keys.len(),
        address_lookup_tables: vec![],
        frame_used: opts.frame_used.map(|p| p.to_string()),
        fee_payer: opts.fee_payer.map(|p| p.to_string()),
        smart_close_applied: opts.smart_close_applied,
        transaction_size_bytes: opts.transaction_size_bytes,
        transaction_config: opts.transaction_config.clone(),
        instructions: inspected,
    }
}

fn collect_static_keys(
    fee_payer: Option<&Pubkey>,
    instructions: &[solana_sdk::instruction::Instruction],
) -> Vec<String> {
    let mut keys = Vec::new();
    let mut seen = HashSet::new();
    let mut add = |pk: String| {
        if seen.insert(pk.clone()) {
            keys.push(pk);
        }
    };
    if let Some(fp) = fee_payer {
        add(fp.to_string());
    }
    for ix in instructions {
        add(ix.program_id.to_string());
        for k in &ix.accounts {
            add(k.pubkey.to_string());
        }
    }
    keys
}

pub fn inspect_versioned_transaction(
    tx: &VersionedTransaction,
    lookup_tables: &[AddressLookupTableAccount],
    opts: &InspectOpts,
) -> TxInspection {
    let VersionedMessage::V0(v0) = &tx.message else {
        return TxInspection {
            version: 0,
            num_instructions: 0,
            static_account_keys: 0,
            loaded_writable_accounts: 0,
            loaded_readonly_accounts: 0,
            total_account_keys: 0,
            address_lookup_tables: opts.address_lookup_table_addresses.clone(),
            frame_used: opts.frame_used.map(|p| p.to_string()),
            fee_payer: opts.fee_payer.map(|p| p.to_string()),
            smart_close_applied: opts.smart_close_applied,
            transaction_size_bytes: opts.transaction_size_bytes,
            transaction_config: None,
            instructions: vec![],
        };
    };

    let loaded = load_addresses_from_tables(&v0.address_table_lookups, lookup_tables);
    let reserved: HashSet<Pubkey> = HashSet::new();
    let loaded_msg = LoadedMessage::new_borrowed(v0, &loaded, &reserved);

    let static_key_count = v0.account_keys.len();
    let loaded_writable = v0
        .address_table_lookups
        .iter()
        .map(|l| l.writable_indexes.len())
        .sum();
    let loaded_readonly = v0
        .address_table_lookups
        .iter()
        .map(|l| l.readonly_indexes.len())
        .sum();

    let account_keys = loaded_msg.account_keys();
    let alt_table_addresses = build_alt_table_address_set(lookup_tables);

    let instructions: Vec<TxInstructionInspection> = v0
        .instructions
        .iter()
        .enumerate()
        .map(|(index, ix)| {
            let program_id = pubkey_str(account_keys.get(ix.program_id_index as usize).expect("program id"));
            let data = &ix.data;
            let accounts = ix
                .accounts
                .iter()
                .map(|&key_index| {
                    let key_index = key_index as usize;
                    let flags = account_meta_flags(
                        key_index,
                        static_key_count,
                        loaded_writable,
                        &v0.header,
                    );
                    let pubkey = pubkey_str(account_keys.get(key_index).expect("account key"));
                    let resolution = account_key_resolution(key_index, static_key_count, loaded_writable);
                    let alt_loaded = resolution != "static";
                    let in_alt_table = alt_table_addresses.contains(&pubkey);
                    TxAccountInspection {
                        index: key_index,
                        pubkey,
                        is_signer: flags.0,
                        is_writable: flags.1,
                        alt_loaded,
                        resolution: resolution.to_string(),
                        in_alt_table_unused: !alt_loaded && in_alt_table,
                    }
                })
                .collect();

            TxInstructionInspection {
                index,
                program_id: program_id.clone(),
                program_label: program_label(&program_id, &opts.ifx_program_id),
                hint: instruction_hint(&program_id, data, &opts.ifx_program_id),
                accounts,
                data_hex: bytes_to_hex(data),
                data_base64: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    data,
                ),
                data_length: data.len(),
            }
        })
        .collect();

    TxInspection {
        version: 0,
        num_instructions: instructions.len(),
        static_account_keys: static_key_count,
        loaded_writable_accounts: loaded_writable,
        loaded_readonly_accounts: loaded_readonly,
        total_account_keys: account_keys.len(),
        address_lookup_tables: opts.address_lookup_table_addresses.clone(),
        frame_used: opts.frame_used.map(|p| p.to_string()),
        fee_payer: opts.fee_payer.map(|p| p.to_string()),
        smart_close_applied: opts.smart_close_applied,
        transaction_size_bytes: opts.transaction_size_bytes,
        transaction_config: None,
        instructions,
    }
}

fn load_addresses_from_tables(
    lookups: &[MessageAddressTableLookup],
    tables: &[AddressLookupTableAccount],
) -> LoadedAddresses {
    let mut writable = Vec::new();
    let mut readonly = Vec::new();
    for lookup in lookups {
        let table = tables
            .iter()
            .find(|t| t.key == lookup.account_key)
            .unwrap_or_else(|| panic!("missing lookup table {}", lookup.account_key));
        for &i in &lookup.writable_indexes {
            writable.push(table.addresses[i as usize]);
        }
        for &i in &lookup.readonly_indexes {
            readonly.push(table.addresses[i as usize]);
        }
    }
    LoadedAddresses { writable, readonly }
}

fn build_alt_table_address_set(tables: &[AddressLookupTableAccount]) -> HashSet<String> {
    let mut set = HashSet::new();
    for table in tables {
        for addr in &table.addresses {
            set.insert(addr.to_string());
        }
    }
    set
}

fn pubkey_str(key: &Pubkey) -> String {
    key.to_string()
}

fn account_meta_flags(
    key_index: usize,
    static_key_count: usize,
    loaded_writable: usize,
    header: &MessageHeader,
) -> (bool, bool) {
    let num_required_signatures = header.num_required_signatures as usize;
    let num_readonly_signed = header.num_readonly_signed_accounts as usize;
    let num_readonly_unsigned = header.num_readonly_unsigned_accounts as usize;

    if key_index < static_key_count {
        let is_signer = key_index < num_required_signatures;
        if is_signer {
            return (
                true,
                key_index < num_required_signatures.saturating_sub(num_readonly_signed),
            );
        }
        let unsigned_idx = key_index.saturating_sub(num_required_signatures);
        let num_unsigned = static_key_count.saturating_sub(num_required_signatures);
        return (
            false,
            unsigned_idx < num_unsigned.saturating_sub(num_readonly_unsigned),
        );
    }

    let loaded_idx = key_index - static_key_count;
    if loaded_idx < loaded_writable {
        (false, true)
    } else {
        (false, false)
    }
}

fn account_key_resolution(
    key_index: usize,
    static_key_count: usize,
    loaded_writable: usize,
) -> &'static str {
    if key_index < static_key_count {
        "static"
    } else if key_index - static_key_count < loaded_writable {
        "alt-writable"
    } else {
        "alt-readonly"
    }
}

fn bytes_to_hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn program_label(program_id: &str, ifx_program_id: &Pubkey) -> String {
    if program_id == ifx_program_id.to_string() {
        return "Ifx".into();
    }
    match program_id {
        "ComputeBudget111111111111111111111111111111" => "Compute Budget".into(),
        "11111111111111111111111111111111" => "System Program".into(),
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" => "SPL Token".into(),
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL" => "Associated Token".into(),
        "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C" => "Raydium CPMM".into(),
        other => format!("{}…{}", &other[..4], &other[other.len() - 4..]),
    }
}

fn instruction_hint(program_id: &str, data: &[u8], ifx_program_id: &Pubkey) -> Option<String> {
    if program_id == "ComputeBudget111111111111111111111111111111" {
        if data.first() == Some(&2) && data.len() >= 5 {
            let units = u32::from_le_bytes(data[1..5].try_into().ok()?);
            return Some(format!("SetComputeUnitLimit({units})"));
        }
        if data.first() == Some(&3) && data.len() >= 9 {
            let micro = u64::from_le_bytes(data[1..9].try_into().ok()?);
            return Some(format!("SetComputeUnitPrice({micro} µL/CU)"));
        }
    }
    if program_id == "11111111111111111111111111111111" && data.len() >= 4 {
        let kind = u32::from_le_bytes(data[0..4].try_into().ok()?);
        if kind == 2 {
            return Some("Transfer".into());
        }
    }
    if program_id == "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA" {
        if data.first() == Some(&17) {
            return Some("SyncNative".into());
        }
        if data.first() == Some(&3) {
            return Some("Transfer".into());
        }
    }
    if program_id == "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C" && data.len() >= 8 {
        return Some("swap_base_input".into());
    }
    if program_id == ifx_program_id.to_string() {
        if let Some(label) = ifx_ix_hint(data) {
            return Some(label.to_string());
        }
    }
    None
}
