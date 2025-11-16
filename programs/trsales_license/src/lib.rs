use anchor_lang::context::Context as ProgramContext;
use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

pub const CONFIG_SEED: &[u8] = b"config";
pub const PROFILE_SEED: &[u8] = b"profile";
pub const REWARD_SEED: &[u8] = b"reward";
pub const ORDER_SEED: &[u8] = b"order";

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitializeArgs {
    pub admin: Pubkey,
    pub usdc_mint: Pubkey,
    pub collection_mint: Pubkey,
    pub sol_price: u64,
    pub usdc_price: u64,
}

declare_id!("Trsa1esReferr4l1ju8GhhQXUfdViQspuWqX9u9KQk8k");

#[program]
pub mod trsales_license {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.admin = args.admin;
        config.usdc_mint = args.usdc_mint;
        config.collection_mint = args.collection_mint;
        config.treasury_sol = ctx.accounts.treasury_sol.key();
        config.treasury_usdc = ctx.accounts.treasury_usdc.key();
        config.sol_price = args.sol_price;
        config.usdc_price = args.usdc_price;
        config.bump = ctx.bumps.config;

        Ok(())
    }

    pub fn register_user(ctx: Context<RegisterUser>) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        profile.user = ctx.accounts.user.key();
        profile.has_referrer = false;
        profile.referrer = Pubkey::default();
        profile.bump = ctx.bumps.profile;
        profile.minted_nft = false;

        let reward_vault = &mut ctx.accounts.reward_vault;
        reward_vault.user = ctx.accounts.user.key();
        reward_vault.claimable_sol = 0;
        reward_vault.claimable_usdc = 0;
        reward_vault.bump = ctx.bumps.reward_vault;

        Ok(())
    }

    pub fn set_referrer(ctx: Context<SetReferrer>, parent: Pubkey) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        require!(
            profile.user == ctx.accounts.user.key(),
            TrsalesError::Unauthorized
        );
        require!(profile.user != parent, TrsalesError::SelfReferral);
        require!(!profile.has_referrer, TrsalesError::ReferrerAlreadySet);
        require!(
            ctx.accounts.parent_profile.user == parent,
            TrsalesError::InvalidReferrer
        );

        profile.has_referrer = true;
        profile.referrer = parent;

        Ok(())
    }

    pub fn mint_with_sol<'info>(
        ctx: ProgramContext<'_, '_, '_, 'info, MintWithSol<'info>>,
        order_seed: Vec<u8>,
    ) -> Result<()> {
        require_eq!(order_seed.len(), 16, TrsalesError::InvalidOrderSeed);
        let config = &ctx.accounts.config;
        require_keys_eq!(
            ctx.accounts.treasury_sol.key(),
            config.treasury_sol,
            TrsalesError::InvalidTreasury
        );
        require_keys_eq!(
            ctx.accounts.collection_mint.key(),
            config.collection_mint,
            TrsalesError::CollectionMismatch
        );
        require_keys_eq!(
            ctx.accounts.buyer_profile.user,
            ctx.accounts.buyer.key(),
            TrsalesError::Unauthorized
        );
        require_keys_eq!(
            ctx.accounts.buyer_reward_vault.user,
            ctx.accounts.buyer.key(),
            TrsalesError::Unauthorized
        );

        let order = &mut ctx.accounts.order;
        order.id = order_seed
            .clone()
            .try_into()
            .map_err(|_| TrsalesError::InvalidOrderSeed)?;
        order.bump = ctx.bumps.order;

        distribute_sol(
            ctx.accounts.buyer.as_ref(),
            ctx.accounts.treasury_sol.as_ref(),
            ctx.accounts.system_program.as_ref(),
            ctx.remaining_accounts,
            config.sol_price,
        )?;

        mint_collection_nft(
            &ctx.accounts.config,
            &ctx.accounts.collection_mint,
            &ctx.accounts.buyer_nft_account,
            &ctx.accounts.token_program,
        )?;

        ctx.accounts.buyer_profile.minted_nft = true;

        Ok(())
    }

    pub fn mint_with_usdc<'info>(
        ctx: ProgramContext<'_, '_, '_, 'info, MintWithUsdc<'info>>,
        order_seed: Vec<u8>,
    ) -> Result<()> {
        require_eq!(order_seed.len(), 16, TrsalesError::InvalidOrderSeed);
        let config = &ctx.accounts.config;
        require_keys_eq!(
            ctx.accounts.treasury_sol.key(),
            config.treasury_sol,
            TrsalesError::InvalidTreasury
        );
        require_keys_eq!(
            ctx.accounts.treasury_usdc.mint,
            config.usdc_mint,
            TrsalesError::InvalidTreasury
        );
        require_keys_eq!(
            ctx.accounts.collection_mint.key(),
            config.collection_mint,
            TrsalesError::CollectionMismatch
        );
        require_keys_eq!(
            ctx.accounts.buyer_profile.user,
            ctx.accounts.buyer.key(),
            TrsalesError::Unauthorized
        );
        require_keys_eq!(
            ctx.accounts.buyer_reward_vault.user,
            ctx.accounts.buyer.key(),
            TrsalesError::Unauthorized
        );

        let order = &mut ctx.accounts.order;
        order.id = order_seed
            .clone()
            .try_into()
            .map_err(|_| TrsalesError::InvalidOrderSeed)?;
        order.bump = ctx.bumps.order;

        distribute_usdc(
            ctx.accounts.buyer.as_ref(),
            ctx.accounts.buyer_usdc.as_ref(),
            ctx.accounts.treasury_usdc.as_ref(),
            ctx.accounts.token_program.as_ref(),
            ctx.remaining_accounts,
            config.usdc_price,
        )?;

        mint_collection_nft(
            &ctx.accounts.config,
            &ctx.accounts.collection_mint,
            &ctx.accounts.buyer_nft_account,
            &ctx.accounts.token_program,
        )?;

        ctx.accounts.buyer_profile.minted_nft = true;

        Ok(())
    }

    pub fn claim_sol(ctx: Context<ClaimSol>) -> Result<()> {
        let amount = ctx.accounts.reward_vault.claimable_sol;
        require!(amount > 0, TrsalesError::NothingToClaim);
        require_keys_eq!(
            ctx.accounts.reward_vault.user,
            ctx.accounts.user.key(),
            TrsalesError::Unauthorized
        );

        let bump = ctx.accounts.reward_vault.bump;
        let signer_seeds: &[&[u8]] = &[REWARD_SEED, ctx.accounts.user.key.as_ref(), &[bump]];
        let signer = &[signer_seeds];

        let ix = system_instruction::transfer(
            &ctx.accounts.reward_vault.key(),
            &ctx.accounts.user.key(),
            amount,
        );
        invoke_signed(
            &ix,
            &[
                ctx.accounts.reward_vault.to_account_info(),
                ctx.accounts.user.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer,
        )?;

        ctx.accounts.reward_vault.claimable_sol = 0;
        Ok(())
    }

    pub fn claim_usdc(ctx: Context<ClaimUsdc>) -> Result<()> {
        let amount = ctx.accounts.reward_vault.claimable_usdc;
        require!(amount > 0, TrsalesError::NothingToClaim);
        require_keys_eq!(
            ctx.accounts.reward_vault.user,
            ctx.accounts.user.key(),
            TrsalesError::Unauthorized
        );

        let bump = ctx.accounts.reward_vault.bump;
        let signer_seeds: &[&[u8]] = &[REWARD_SEED, ctx.accounts.user.key.as_ref(), &[bump]];
        let signer = &[signer_seeds];

        let cpi_accounts = Transfer {
            from: ctx.accounts.reward_usdc.to_account_info(),
            to: ctx.accounts.user_usdc.to_account_info(),
            authority: ctx.accounts.reward_vault.to_account_info(),
        };
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                cpi_accounts,
                signer,
            ),
            amount,
        )?;

        ctx.accounts.reward_vault.claimable_usdc = 0;
        Ok(())
    }
}

fn distribute_sol<'info>(
    buyer: &AccountInfo<'info>,
    treasury_sol: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    price: u64,
) -> Result<()> {
    let mut distributed: u64 = 0;
    let mut level = 0;

    for chunk in remaining.chunks(4) {
        if chunk.len() < 4 || level >= 3 {
            break;
        }
        let profile_info = &chunk[0];
        let reward_vault_info = &chunk[1];
        validate_profile_owner(profile_info, reward_vault_info)?;

        let percent = match level {
            0 => 50,
            1 => 30,
            _ => 20,
        };
        let amount = price
            .checked_mul(percent)
            .and_then(|v| v.checked_div(100))
            .ok_or(TrsalesError::MathOverflow)?;

        let ix = system_instruction::transfer(&buyer.key, &reward_vault_info.key, amount);
        invoke(
            &ix,
            &[
                buyer.clone(),
                reward_vault_info.clone(),
                system_program.clone(),
            ],
        )?;

        increment_reward_vault(reward_vault_info, amount, RewardField::Sol)?;
        distributed = distributed
            .checked_add(amount)
            .ok_or(TrsalesError::MathOverflow)?;
        level += 1;
    }

    let remainder = price
        .checked_sub(distributed)
        .ok_or(TrsalesError::MathOverflow)?;
    if remainder > 0 {
        let ix = system_instruction::transfer(&buyer.key, &treasury_sol.key, remainder);
        invoke(
            &ix,
            &[buyer.clone(), treasury_sol.clone(), system_program.clone()],
        )?;
    }

    Ok(())
}

fn distribute_usdc<'info>(
    buyer: &AccountInfo<'info>,
    buyer_usdc: &AccountInfo<'info>,
    treasury_usdc: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    price: u64,
) -> Result<()> {
    let mut distributed: u64 = 0;
    let mut level = 0;

    for chunk in remaining.chunks(4) {
        if chunk.len() < 4 || level >= 3 {
            break;
        }
        let profile_info = &chunk[0];
        let reward_vault_info = &chunk[1];
        let reward_usdc_info = &chunk[2];
        validate_profile_owner(profile_info, reward_vault_info)?;

        let percent = match level {
            0 => 50,
            1 => 30,
            _ => 20,
        };
        let amount = price
            .checked_mul(percent)
            .and_then(|v| v.checked_div(100))
            .ok_or(TrsalesError::MathOverflow)?;

        let cpi_accounts = Transfer {
            from: buyer_usdc.clone(),
            to: reward_usdc_info.clone(),
            authority: buyer.clone(),
        };
        token::transfer(CpiContext::new(token_program.clone(), cpi_accounts), amount)?;

        increment_reward_vault(reward_vault_info, amount, RewardField::Usdc)?;
        distributed = distributed
            .checked_add(amount)
            .ok_or(TrsalesError::MathOverflow)?;
        level += 1;
    }

    let remainder = price
        .checked_sub(distributed)
        .ok_or(TrsalesError::MathOverflow)?;
    if remainder > 0 {
        let cpi_accounts = Transfer {
            from: buyer_usdc.clone(),
            to: treasury_usdc.clone(),
            authority: buyer.clone(),
        };
        token::transfer(
            CpiContext::new(token_program.clone(), cpi_accounts),
            remainder,
        )?;
    }

    Ok(())
}

fn validate_profile_owner(
    profile_info: &AccountInfo,
    reward_vault_info: &AccountInfo,
) -> Result<()> {
    require_keys_eq!(
        *profile_info.owner,
        crate::id(),
        TrsalesError::InvalidProfileAccount
    );
    require_keys_eq!(
        *reward_vault_info.owner,
        crate::id(),
        TrsalesError::InvalidRewardVault
    );

    let profile_user = {
        let data = profile_info.try_borrow_data()?;
        let mut slice: &[u8] = &data;
        let profile = ReferralProfile::try_deserialize(&mut slice)?;
        profile.user
    };
    let reward_user = {
        let data = reward_vault_info.try_borrow_data()?;
        let mut slice: &[u8] = &data;
        let reward = RewardVault::try_deserialize(&mut slice)?;
        reward.user
    };

    require_keys_eq!(profile_user, reward_user, TrsalesError::ProfileMismatch);
    Ok(())
}

enum RewardField {
    Sol,
    Usdc,
}

fn increment_reward_vault(account: &AccountInfo, amount: u64, field: RewardField) -> Result<()> {
    require_keys_eq!(
        *account.owner,
        crate::id(),
        TrsalesError::InvalidRewardVault
    );
    let mut data = account.try_borrow_mut_data()?;
    let mut slice: &[u8] = &data;
    let mut reward = RewardVault::try_deserialize(&mut slice)?;
    match field {
        RewardField::Sol => {
            reward.claimable_sol = reward
                .claimable_sol
                .checked_add(amount)
                .ok_or(TrsalesError::MathOverflow)?;
        }
        RewardField::Usdc => {
            reward.claimable_usdc = reward
                .claimable_usdc
                .checked_add(amount)
                .ok_or(TrsalesError::MathOverflow)?;
        }
    }
    let mut out_slice: &mut [u8] = &mut data;
    reward.serialize(&mut out_slice)?;
    Ok(())
}

fn mint_collection_nft<'info>(
    config: &Account<'info, Config>,
    mint: &Account<'info, Mint>,
    destination: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    let seeds: &[&[u8]] = &[CONFIG_SEED, &[config.bump]];
    let signer = &[seeds];
    let cpi_accounts = MintTo {
        mint: mint.to_account_info(),
        to: destination.to_account_info(),
        authority: config.to_account_info(),
    };
    token::mint_to(
        CpiContext::new_with_signer(token_program.to_account_info(), cpi_accounts, signer),
        1,
    )
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = payer,
        seeds = [CONFIG_SEED],
        bump,
        space = Config::LEN
    )]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK: stored in config
    pub treasury_sol: UncheckedAccount<'info>,
    pub treasury_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub collection_mint: Account<'info, Mint>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct RegisterUser<'info> {
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init,
        payer = payer,
        seeds = [PROFILE_SEED, user.key().as_ref()],
        bump,
        space = ReferralProfile::LEN
    )]
    pub profile: Account<'info, ReferralProfile>,
    #[account(
        init,
        payer = payer,
        seeds = [REWARD_SEED, user.key().as_ref()],
        bump,
        space = RewardVault::LEN
    )]
    pub reward_vault: Account<'info, RewardVault>,
    /// CHECK: Provided for compatibility with clients
    #[account(mut)]
    pub reward_usdc: UncheckedAccount<'info>,
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetReferrer<'info> {
    #[account(mut, has_one = user)]
    pub profile: Account<'info, ReferralProfile>,
    pub user: Signer<'info>,
    pub parent_profile: Account<'info, ReferralProfile>,
}

#[derive(Accounts)]
#[instruction(order_seed: Vec<u8>)]
pub struct MintWithSol<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    /// CHECK: validated against config
    #[account(mut)]
    pub treasury_sol: UncheckedAccount<'info>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(
        init,
        payer = buyer,
        seeds = [ORDER_SEED, order_seed.as_slice()],
        bump,
        space = Order::LEN
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub buyer_profile: Account<'info, ReferralProfile>,
    #[account(mut)]
    pub buyer_reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub collection_mint: Account<'info, Mint>,
    #[account(mut)]
    pub buyer_nft_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(order_seed: Vec<u8>)]
pub struct MintWithUsdc<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    /// CHECK: validated against config
    #[account(mut)]
    pub treasury_sol: UncheckedAccount<'info>,
    #[account(mut)]
    pub treasury_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut)]
    pub buyer_usdc: Account<'info, TokenAccount>,
    #[account(
        init,
        payer = buyer,
        seeds = [ORDER_SEED, order_seed.as_slice()],
        bump,
        space = Order::LEN
    )]
    pub order: Account<'info, Order>,
    #[account(mut)]
    pub buyer_profile: Account<'info, ReferralProfile>,
    #[account(mut)]
    pub buyer_reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub collection_mint: Account<'info, Mint>,
    #[account(mut)]
    pub buyer_nft_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimSol<'info> {
    #[account(mut, seeds = [REWARD_SEED, reward_vault.user.as_ref()], bump = reward_vault.bump)]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimUsdc<'info> {
    #[account(mut, seeds = [REWARD_SEED, reward_vault.user.as_ref()], bump = reward_vault.bump)]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub reward_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub user_usdc: Account<'info, TokenAccount>,
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

#[account]
pub struct Config {
    pub admin: Pubkey,
    pub usdc_mint: Pubkey,
    pub collection_mint: Pubkey,
    pub treasury_sol: Pubkey,
    pub treasury_usdc: Pubkey,
    pub sol_price: u64,
    pub usdc_price: u64,
    pub bump: u8,
}

impl Config {
    pub const LEN: usize = 8 + 32 * 5 + 8 * 2 + 1;
}

#[account]
pub struct ReferralProfile {
    pub user: Pubkey,
    pub has_referrer: bool,
    pub referrer: Pubkey,
    pub minted_nft: bool,
    pub bump: u8,
}

impl ReferralProfile {
    pub const LEN: usize = 8 + 32 + 1 + 32 + 1 + 1;
}

#[account]
pub struct RewardVault {
    pub user: Pubkey,
    pub claimable_sol: u64,
    pub claimable_usdc: u64,
    pub bump: u8,
}

impl RewardVault {
    pub const LEN: usize = 8 + 32 + 8 + 8 + 1;
}

#[account]
pub struct Order {
    pub id: [u8; 16],
    pub bump: u8,
}

impl Order {
    pub const LEN: usize = 8 + 16 + 1;
}

#[error_code]
pub enum TrsalesError {
    #[msg("Invalid treasury account provided")]
    InvalidTreasury,
    #[msg("Order seed must be 16 bytes")]
    InvalidOrderSeed,
    #[msg("Mathematical overflow")]
    MathOverflow,
    #[msg("Unauthorized action")]
    Unauthorized,
    #[msg("Referrer already set")]
    ReferrerAlreadySet,
    #[msg("Invalid referrer provided")]
    InvalidReferrer,
    #[msg("Cannot refer yourself")]
    SelfReferral,
    #[msg("Collection mint mismatch")]
    CollectionMismatch,
    #[msg("Nothing to claim")]
    NothingToClaim,
    #[msg("Invalid reward vault provided")]
    InvalidRewardVault,
    #[msg("Invalid profile account provided")]
    InvalidProfileAccount,
    #[msg("Profile and reward owner mismatch")]
    ProfileMismatch,
}
