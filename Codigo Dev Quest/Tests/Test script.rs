// Use the program test crate for a local test environment.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::instruction::{AccountMeta, Instruction};
use anchor_lang::solana_program::system_program;
use anchor_lang::solana_program::{pubkey::Pubkey, system_instruction};
use anchor_lang::{AnchorDeserialize, AnchorSerialize};
use solana_program_test::{processor, tokio, BanksClient, ProgramTest};
use solana_sdk::signature::{Keypair, Signer};
use solana_sdk::transaction::{Transaction, Message};
use solana_sdk::transport::TransportError;

// Bring in the types from the program we are testing.
use multisig::{
    MultiSigWallet, MultiSigError, TransactionProposal,
    initialize_wallet, create_proposal, approve_proposal, execute_proposal
};

// Define a simple mock program for testing CPIs.
#[derive(Accounts)]
pub struct MockInstructionContext<'info> {
    #[account(mut)]
    pub target_account: AccountInfo<'info>,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct MockInstructionData {
    pub value: u64,
}

fn process_mock_instruction(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> Result<()> {
    let mut data = instruction_data;
    let mock_data: MockInstructionData = AnchorDeserialize::deserialize(&mut data)?;
    msg!("Mock program received value: {}", mock_data.value);
    Ok(())
}

// ----------------------
// Helper Functions
// ----------------------

/// Sets up the basic program test environment.
async fn setup_test_environment() -> (BanksClient, Keypair, Pubkey) {
    let mut program_test = ProgramTest::new(
        "multisig",
        multisig::ID,
        processor!(multisig::entry),
    );
    let mock_program_id = Pubkey::new_unique();
    program_test.add_program("mock_program", mock_program_id, Some(process_mock_instruction));
    let (banks_client, payer, recent_blockhash) = program_test.start().await;
    (banks_client, payer, mock_program_id)
}

/// A helper function to build and send a transaction.
async fn build_and_send_tx(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    signers: &[&Keypair],
    instructions: Vec<Instruction>,
    recent_blockhash: Hash,
) -> Result<(), TransportError> {
    let mut transaction = Transaction::new_with_payer(
        &instructions,
        Some(&payer.pubkey()),
    );
    transaction.sign(signers, recent_blockhash);
    banks_client.process_transaction(transaction).await
}

/// Initializes a new multi-sig wallet for testing.
async fn initialize_test_wallet(
    banks_client: &mut BanksClient,
    payer: &Keypair,
    recent_blockhash: Hash,
    signers_vec: Vec<Pubkey>,
    threshold: u64,
) -> Result<Pubkey, TransportError> {
    let (wallet_pda, _wallet_bump) = Pubkey::find_program_address(&[b"multisig"], &multisig::ID);
    
    let instruction = multisig::instruction::InitializeWallet {
        signers: signers_vec,
        threshold,
    };
    
    let ix = Instruction {
        program_id: multisig::ID,
        accounts: vec![
            AccountMeta::new(wallet_pda, false),
            AccountMeta::new(payer.pubkey(), true),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: instruction.data(),
    };
    
    build_and_send_tx(banks_client, payer, &[payer], vec![ix], recent_blockhash).await?;
    Ok(wallet_pda)
}

// ----------------------
// Tests
// ----------------------

#[tokio::test]
async fn test_initialize_wallet_success() {
    let (mut banks_client, payer, _mock_program_id) = setup_test_environment().await;
    let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();

    let signer1 = Keypair::new();
    let signer2 = Keypair::new();
    let signer3 = Keypair::new();
    let signers_vec = vec![signer1.pubkey(), signer2.pubkey(), signer3.pubkey()];
    let threshold = 2;

    let wallet_pda = initialize_test_wallet(
        &mut banks_client,
        &payer,
        recent_blockhash,
        signers_vec.clone(),
        threshold,
    ).await.unwrap();

    let wallet_account = banks_client.get_account(wallet_pda).await.unwrap().unwrap();
    let wallet_data = MultiSigWallet::try_from_slice(&wallet_account.data[8..]).unwrap();
    assert_eq!(wallet_data.signers, signers_vec);
    assert_eq!(wallet_data.threshold, threshold);
    assert_eq!(wallet_data.proposal_count, 0);
}

#[tokio::test]
async fn test_create_proposal_success() {
    let (mut banks_client, payer, mock_program_id) = setup_test_environment().await;
    let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();

    let signer1 = Keypair::new();
    let signer2 = Keypair::new();
    let signers_vec = vec![signer1.pubkey(), signer2.pubkey()];
    let threshold = 1;

    let wallet_pda = initialize_test_wallet(
        &mut banks_client,
        &payer,
        recent_blockhash,
        signers_vec,
        threshold,
    ).await.unwrap();
    
    // Create a mock instruction to be proposed.
    let mock_instruction_data = multisig::instruction::MockInstructionData { value: 42 };
    let instruction_accounts = vec![
        AccountMeta::new(Pubkey::new_unique(), false),
    ];
    let (proposal_pda, _proposal_bump) = Pubkey::find_program_address(
        &[b"proposal", wallet_pda.as_ref(), &1_u64.to_le_bytes()],
        &multisig::ID,
    );
    
    let proposal_instruction_data = multisig::instruction::CreateProposal {
        instruction_data: mock_instruction_data.data(),
        instruction_program_id: mock_program_id,
        instruction_accounts: instruction_accounts.clone(),
    };
    
    let ix = Instruction {
        program_id: multisig::ID,
        accounts: vec![
            AccountMeta::new(wallet_pda, false),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(signer1.pubkey(), true),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: proposal_instruction_data.data(),
    };
    
    build_and_send_tx(&mut banks_client, &payer, &[&signer1], vec![ix], recent_blockhash)
        .await
        .unwrap();

    let proposal_account = banks_client.get_account(proposal_pda).await.unwrap().unwrap();
    let proposal_data = TransactionProposal::try_from_slice(&proposal_account.data[8..]).unwrap();
    
    assert_eq!(proposal_data.multi_sig, wallet_pda);
    assert_eq!(proposal_data.proposer, signer1.pubkey());
    assert_eq!(proposal_data.index, 1);
    assert_eq!(proposal_data.approvals, vec![signer1.pubkey()]);
    assert!(!proposal_data.executed);
}

#[tokio::test]
async fn test_approve_and_execute_success() {
    let (mut banks_client, payer, mock_program_id) = setup_test_environment().await;
    let recent_blockhash = banks_client.get_recent_blockhash().await.unwrap();

    let signer1 = Keypair::new();
    let signer2 = Keypair::new();
    let signers_vec = vec![signer1.pubkey(), signer2.pubkey()];
    let threshold = 2;

    let wallet_pda = initialize_test_wallet(
        &mut banks_client,
        &payer,
        recent_blockhash,
        signers_vec.clone(),
        threshold,
    ).await.unwrap();
    
    // Create a mock instruction to be proposed.
    let mock_instruction_data = multisig::instruction::MockInstructionData { value: 42 };
    let target_account_keypair = Keypair::new();
    let instruction_accounts = vec![
        AccountMeta::new(target_account_keypair.pubkey(), false),
    ];
    let (proposal_pda, _proposal_bump) = Pubkey::find_program_address(
        &[b"proposal", wallet_pda.as_ref(), &1_u64.to_le_bytes()],
        &multisig::ID,
    );
    
    let proposal_instruction_data = multisig::instruction::CreateProposal {
        instruction_data: mock_instruction_data.data(),
        instruction_program_id: mock_program_id,
        instruction_accounts: instruction_accounts.clone(),
    };
    
    let ix_proposal = Instruction {
        program_id: multisig::ID,
        accounts: vec![
            AccountMeta::new(wallet_pda, false),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(signer1.pubkey(), true),
            AccountMeta::new_readonly(system_program::ID, false),
        ],
        data: proposal_instruction_data.data(),
    };
    
    build_and_send_tx(&mut banks_client, &payer, &[&signer1], vec![ix_proposal], recent_blockhash)
        .await
        .unwrap();

    // Approve the proposal with signer2.
    let ix_approve = Instruction {
        program_id: multisig::ID,
        accounts: vec![
            AccountMeta::new(wallet_pda, false),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(signer2.pubkey(), true),
        ],
        data: approve_proposal::ID.to_vec(),
    };

    build_and_send_tx(&mut banks_client, &payer, &[&signer2], vec![ix_approve], recent_blockhash)
        .await
        .unwrap();

    let proposal_account = banks_client.get_account(proposal_pda).await.unwrap().unwrap();
    let proposal_data = TransactionProposal::try_from_slice(&proposal_account.data[8..]).unwrap();
    assert_eq!(proposal_data.approvals.len(), 2);

    // Execute the proposal.
    let ix_execute = Instruction {
        program_id: multisig::ID,
        accounts: vec![
            AccountMeta::new(wallet_pda, false),
            AccountMeta::new(proposal_pda, false),
            AccountMeta::new(signer1.pubkey(), true),
            AccountMeta::new_readonly(mock_program_id, false),
            AccountMeta::new(target_account_keypair.pubkey(), false),
        ],
        data: execute_proposal::ID.to_vec(),
    };

    build_and_send_tx(&mut banks_client, &payer, &[&signer1], vec![ix_execute], recent_blockhash)
        .await
        .unwrap();

    let proposal_account = banks_client.get_account(proposal_pda).await.unwrap().unwrap();
    let proposal_data = TransactionProposal::try_from_slice(&proposal_account.data[8..]).unwrap();
    assert!(proposal_data.executed);
}
