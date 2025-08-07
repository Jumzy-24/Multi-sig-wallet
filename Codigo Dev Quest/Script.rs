// Use the Anchor framework for Solana smart contracts.
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::Instruction,
    program::invoke_signed,
    pubkey::Pubkey,
};
use std::collections::BTreeSet;

declare_id!("8g3mXgJ3r1K1y2f4b5c6d7e8f9g0h1i2j3k4l5m6n7o8p");

#[program]
pub mod multisig {
    use super::*;

    /// Initializes a new MultiSig Wallet.
    pub fn initialize_wallet(
        ctx: Context<InitializeWallet>,
        signers: Vec<Pubkey>,
        threshold: u64,
    ) -> Result<()> {
        let wallet = &mut ctx.accounts.wallet;
        
        require!(
            signers.len() as u64 >= threshold,
            MultiSigError::ThresholdTooHigh
        );
        let unique_signers: BTreeSet<Pubkey> = signers.iter().cloned().collect();
        require!(
            unique_signers.len() == signers.len(),
            MultiSigError::DuplicateSigner
        );

        wallet.signers = signers;
        wallet.threshold = threshold;
        wallet.proposal_count = 0;
        wallet.bump = *ctx.bumps.get("wallet").unwrap();
        
        emit!(WalletInitialized {
            wallet: wallet.key(),
            signers: wallet.signers.clone(),
            threshold: wallet.threshold,
        });

        Ok(())
    }

    /// Creates a new `TransactionProposal`.
    pub fn create_proposal(
        ctx: Context<CreateProposal>,
        instruction_data: Vec<u8>,
        instruction_program_id: Pubkey,
        instruction_accounts: Vec<AccountMeta>,
    ) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let wallet = &mut ctx.accounts.wallet;

        require!(
            wallet.signers.contains(&ctx.accounts.proposer.key()),
            MultiSigError::InvalidSigner
        );

        let instruction = Instruction {
            program_id: instruction_program_id,
            accounts: instruction_accounts,
            data: instruction_data,
        };

        proposal.multi_sig = wallet.key();
        proposal.proposer = ctx.accounts.proposer.key();
        proposal.instruction = instruction;
        proposal.approvals.push(ctx.accounts.proposer.key());
        proposal.executed = false;
        proposal.bump = *ctx.bumps.get("proposal").unwrap();

        wallet.proposal_count += 1;
        proposal.index = wallet.proposal_count;
        
        emit!(ProposalCreated {
            proposal: proposal.key(),
            proposer: proposal.proposer,
            index: proposal.index,
        });

        Ok(())
    }

    /// Approves an existing `TransactionProposal`.
    pub fn approve_proposal(ctx: Context<ApproveProposal>) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let wallet = &ctx.accounts.wallet;
        
        require!(
            wallet.signers.contains(&ctx.accounts.approver.key()),
            MultiSigError::InvalidSigner
        );
        require!(!proposal.executed, MultiSigError::AlreadyExecuted);
        require!(
            !proposal.approvals.contains(&ctx.accounts.approver.key()),
            MultiSigError::AlreadyApproved
        );

        proposal.approvals.push(ctx.accounts.approver.key());
        
        emit!(ProposalApproved {
            proposal: proposal.key(),
            approver: ctx.accounts.approver.key(),
            approvals_needed: wallet.threshold,
            current_approvals: proposal.approvals.len() as u64,
        });

        Ok(())
    }

    /// Executes a `TransactionProposal`.
    pub fn execute_proposal(ctx: Context<ExecuteProposal>) -> Result<()> {
        let proposal = &mut ctx.accounts.proposal;
        let wallet = &ctx.accounts.wallet;

        require!(!proposal.executed, MultiSigError::AlreadyExecuted);
        require!(
            proposal.approvals.len() as u64 >= wallet.threshold,
            MultiSigError::NotEnoughApprovals
        );

        proposal.executed = true;

        let mut account_infos = Vec::new();
        account_infos.push(ctx.accounts.instruction_program.clone());
        for account in ctx.remaining_accounts.iter() {
            account_infos.push(account.clone());
        }
        
        let wallet_seeds = &[b"multisig", &[wallet.bump]];
        let wallet_signer = &[&wallet_seeds[..]];

        invoke_signed(
            &proposal.instruction,
            &account_infos,
            wallet_signer
        )?;
        
        emit!(ProposalExecuted {
            proposal: proposal.key(),
            index: proposal.index,
            instruction_program: proposal.instruction.program_id,
        });
        
        Ok(())
    }
}

// ----------------------
// Account Contexts
// ----------------------

#[derive(Accounts)]
#[instruction(signers: Vec<Pubkey>, threshold: u64)]
pub struct InitializeWallet<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + 8 + 32 + (32 * signers.len()) + 1,
        seeds = [b"multisig"],
        bump
    )]
    pub wallet: Account<'info, MultiSigWallet>,
    
    #[account(mut)]
    pub payer: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateProposal<'info> {
    #[account(
        mut,
        seeds = [b"multisig"],
        bump = wallet.bump,
    )]
    pub wallet: Account<'info, MultiSigWallet>,
    
    #[account(
        init,
        payer = proposer,
        space = 8 + 8 + 32 + 32 + 1 + 8 + 1024,
        seeds = [b"proposal", wallet.key().as_ref(), wallet.proposal_count.to_le_bytes().as_ref()],
        bump
    )]
    pub proposal: Account<'info, TransactionProposal>,
    
    #[account(mut)]
    pub proposer: Signer<'info>,
    
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ApproveProposal<'info> {
    #[account(
        seeds = [b"multisig"],
        bump = wallet.bump,
    )]
    pub wallet: Account<'info, MultiSigWallet>,
    
    #[account(
        mut,
        has_one = wallet,
        seeds = [b"proposal", wallet.key().as_ref(), proposal.index.to_le_bytes().as_ref()],
        bump = proposal.bump
    )]
    pub proposal: Account<'info, TransactionProposal>,
    
    #[account(mut)]
    pub approver: Signer<'info>,
}

#[derive(Accounts)]
pub struct ExecuteProposal<'info> {
    #[account(
        seeds = [b"multisig"],
        bump = wallet.bump,
    )]
    pub wallet: Account<'info, MultiSigWallet>,
    
    #[account(
        mut,
        has_one = wallet,
        seeds = [b"proposal", wallet.key().as_ref(), proposal.index.to_le_bytes().as_ref()],
        bump = proposal.bump
    )]
    pub proposal: Account<'info, TransactionProposal>,
    
    pub executor: Signer<'info>,
    
    pub instruction_program: AccountInfo<'info>,
}

// ----------------------
// Account Structs
// ----------------------

#[account]
pub struct MultiSigWallet {
    pub signers: Vec<Pubkey>,
    pub threshold: u64,
    pub proposal_count: u64,
    pub bump: u8,
}

#[account]
pub struct TransactionProposal {
    pub multi_sig: Pubkey,
    pub proposer: Pubkey,
    pub index: u64,
    pub instruction: Instruction,
    pub approvals: Vec<Pubkey>,
    pub executed: bool,
    pub bump: u8,
}

// ----------------------
// Events
// ----------------------

#[event]
pub struct WalletInitialized {
    wallet: Pubkey,
    signers: Vec<Pubkey>,
    threshold: u64,
}

#[event]
pub struct ProposalCreated {
    proposal: Pubkey,
    proposer: Pubkey,
    index: u64,
}

#[event]
pub struct ProposalApproved {
    proposal: Pubkey,
    approver: Pubkey,
    approvals_needed: u64,
    current_approvals: u64,
}

#[event]
pub struct ProposalExecuted {
    proposal: Pubkey,
    index: u64,
    instruction_program: Pubkey,
}

// ----------------------
// Error Handling
// ----------------------

#[error_code]
pub enum MultiSigError {
    #[msg("The number of signers is less than the required threshold.")]
    ThresholdTooHigh,
    #[msg("The provided approver is not a valid signer for this wallet.")]
    InvalidSigner,
    #[msg("The proposal has already been executed.")]
    AlreadyExecuted,
    #[msg("This signer has already approved the proposal.")]
    AlreadyApproved,
    #[msg("The number of approvals does not meet the required threshold.")]
    NotEnoughApprovals,
    #[msg("The provided signers contain duplicates.")]
    DuplicateSigner,
}
