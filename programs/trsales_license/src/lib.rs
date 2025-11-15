use anchor_lang::prelude::*;
use anchor_lang::prelude::{AccountDeserialize, AccountSerialize};
use anchor_lang::system_program;
use anchor_spl::associated_token::{get_associated_token_address, AssociatedToken};
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

pub const CONFIG_SEED: &[u8] = b"config";
pub const PROFILE_SEED: &[u8] = b"profile";
pub const REWARD_SEED: &[u8] = b"reward";
pub const ORDER_SEED: &[u8] = b"order";

const MAX_REFERRAL_DEPTH: usize = 3;
const BASIS_POINTS: u64 = 10_000;
const REFERRAL_BPS: [u64; MAX_REFERRAL_DEPTH] = [5_000, 3_000, 2_000];

declare_id!("Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS");

#[program]
pub mod trsales_license {
    use super::*;
    use std::convert::TryInto;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        require!(!args.admin.eq(&Pubkey::default()), ErrorCode::InvalidAdmin);
        require!(args.sol_price > 0, ErrorCode::InvalidPrice);
        require!(args.usdc_price > 0, ErrorCode::InvalidPrice);
        require_keys_eq!(
            args.collection_mint,
            ctx.accounts.collection_mint.key(),
            ErrorCode::InvalidCollectionMint
        );
        require_keys_eq!(
            ctx.accounts.treasury_usdc.mint,
            args.usdc_mint,
            ErrorCode::InvalidTreasuryAccount
        );
        require_keys_eq!(
            ctx.accounts.treasury_usdc.owner,
            args.admin,
            ErrorCode::InvalidTreasuryAccount
        );

        let config = &mut ctx.accounts.config;
        config.admin = args.admin;
        config.treasury_sol = ctx.accounts.treasury_sol.key();
        config.treasury_usdc = ctx.accounts.treasury_usdc.key();
        config.collection_mint = ctx.accounts.collection_mint.key();
        config.usdc_mint = args.usdc_mint;
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

        let reward_vault = &mut ctx.accounts.reward_vault;
        reward_vault.user = ctx.accounts.user.key();
        reward_vault.claimable_sol = 0;
        reward_vault.claimable_usdc = 0;
        reward_vault.bump = ctx.bumps.reward_vault;

        Ok(())
    }

    pub fn set_referrer(ctx: Context<SetReferrer>, parent: Pubkey) -> Result<()> {
        require_keys_neq!(ctx.accounts.user.key(), parent, ErrorCode::InvalidReferrer);

        let profile = &mut ctx.accounts.profile;
        require!(!profile.has_referrer, ErrorCode::ReferrerAlreadySet);

        let parent_profile = &ctx.accounts.parent_profile;
        require_keys_eq!(parent_profile.user, parent, ErrorCode::InvalidReferrer);

        profile.has_referrer = true;
        profile.referrer = parent;

        Ok(())
    }

    pub fn mint_with_sol<'info>(
        ctx: Context<'_, '_, '_, 'info, MintWithSol<'info>>,
        order_id: Vec<u8>,
    ) -> Result<()> {
        require!(order_id.len() == 16, ErrorCode::InvalidOrderId);

        require!(!ctx.accounts.order.used, ErrorCode::OrderAlreadyUsed);

        let buyer_key = ctx.accounts.buyer.key();

        require_keys_eq!(
            ctx.accounts.buyer_profile.user,
            buyer_key,
            ErrorCode::ProfileMismatch
        );

        require_keys_eq!(
            ctx.accounts.buyer_reward_vault.user,
            buyer_key,
            ErrorCode::ProfileMismatch
        );

        let starting_referrer = if ctx.accounts.buyer_profile.has_referrer {
            Some(ctx.accounts.buyer_profile.referrer)
        } else {
            None
        };
        let sol_price = ctx.accounts.config.sol_price;
        let mut distributed: u64 = 0;
        let buyer_info = ctx.accounts.buyer.to_account_info();
        let system_program_info = ctx.accounts.system_program.to_account_info();
        let treasury_sol_info = ctx.accounts.treasury_sol.to_account_info();

        let mut used_accounts = 0;
        let mut expected_referrer = starting_referrer;

        for depth in 0..MAX_REFERRAL_DEPTH {
            let Some(referrer) = expected_referrer else {
                break;
            };

            let start = used_accounts;
            let end = start + 4;
            if end > ctx.remaining_accounts.len() {
                return err!(ErrorCode::MissingReferralAccounts);
            }

            let (node, next_referrer) = validate_referral(
                ctx.program_id,
                &ctx.accounts.collection_mint.key(),
                &ctx.accounts.config.usdc_mint,
                referrer,
                ctx.remaining_accounts[start].clone(),
                ctx.remaining_accounts[start + 1].clone(),
                ctx.remaining_accounts[start + 2].clone(),
                &ctx.remaining_accounts[start + 3],
            )?;

            used_accounts = end;
            expected_referrer = next_referrer;

            let ReferralNode {
                reward_vault_info,
                reward_usdc_info: _,
                reward_vault,
            } = node;

            let share = sol_price
                .checked_mul(REFERRAL_BPS[depth])
                .ok_or(ErrorCode::MathOverflow)?
                .checked_div(BASIS_POINTS)
                .ok_or(ErrorCode::MathOverflow)?;
            if share == 0 {
                continue;
            }

            system_program::transfer(
                CpiContext::new(
                    system_program_info.clone(),
                    system_program::Transfer {
                        from: buyer_info.clone(),
                        to: reward_vault_info.clone(),
                    },
                ),
                share,
            )?;

            let mut updated_vault = reward_vault;
            updated_vault.claimable_sol = updated_vault
                .claimable_sol
                .checked_add(share)
                .ok_or(ErrorCode::MathOverflow)?;
            distributed = distributed
                .checked_add(share)
                .ok_or(ErrorCode::MathOverflow)?;

            {
                let mut data = reward_vault_info.try_borrow_mut_data()?;
                let mut slice: &mut [u8] = &mut data;
                updated_vault.try_serialize(&mut slice)?;
            }
        }

        if used_accounts != ctx.remaining_accounts.len() {
            return err!(ErrorCode::TooManyReferralAccounts);
        }

        let remaining = sol_price
            .checked_sub(distributed)
            .ok_or(ErrorCode::MathOverflow)?;

        if remaining > 0 {
            system_program::transfer(
                CpiContext::new(
                    system_program_info,
                    system_program::Transfer {
                        from: buyer_info,
                        to: treasury_sol_info,
                    },
                ),
                remaining,
            )?;
        }

        mint_collection_nft(
            ctx.accounts.config.bump,
            ctx.accounts.collection_mint.to_account_info(),
            ctx.accounts.buyer_nft_account.to_account_info(),
            ctx.accounts.config.to_account_info(),
            ctx.accounts.token_program.to_account_info(),
        )?;

        let order = &mut ctx.accounts.order;
        order.used = true;
        order.seed = order_id
            .as_slice()
            .try_into()
            .map_err(|_| ErrorCode::InvalidOrderId)?;
        order.buyer = buyer_key;
        order.bump =
            Pubkey::find_program_address(&[ORDER_SEED, order.seed.as_ref()], ctx.program_id).1;

        Ok(())
    }

    pub fn mint_with_usdc<'info>(
        ctx: Context<'_, '_, '_, 'info, MintWithUsdc<'info>>,
        order_id: Vec<u8>,
    ) -> Result<()> {
        require!(order_id.len() == 16, ErrorCode::InvalidOrderId);

        require!(!ctx.accounts.order.used, ErrorCode::OrderAlreadyUsed);

        let buyer_key = ctx.accounts.buyer.key();
        require_keys_eq!(
            ctx.accounts.buyer_profile.user,
            buyer_key,
            ErrorCode::ProfileMismatch
        );

        require_keys_eq!(
            ctx.accounts.buyer_reward_vault.user,
            buyer_key,
            ErrorCode::ProfileMismatch
        );

        let starting_referrer = if ctx.accounts.buyer_profile.has_referrer {
            Some(ctx.accounts.buyer_profile.referrer)
        } else {
            None
        };
        let usdc_price = ctx.accounts.config.usdc_price;
        let mut distributed: u64 = 0;
        let buyer_info = ctx.accounts.buyer.to_account_info();
        let buyer_usdc_info = ctx.accounts.buyer_usdc.to_account_info();
        let token_program_info = ctx.accounts.token_program.to_account_info();
        let treasury_usdc_info = ctx.accounts.treasury_usdc.to_account_info();

        let mut used_accounts = 0;
        let mut expected_referrer = starting_referrer;

        for depth in 0..MAX_REFERRAL_DEPTH {
            let Some(referrer) = expected_referrer else {
                break;
            };

            let start = used_accounts;
            let end = start + 4;
            if end > ctx.remaining_accounts.len() {
                return err!(ErrorCode::MissingReferralAccounts);
            }

            let (node, next_referrer) = validate_referral(
                ctx.program_id,
                &ctx.accounts.collection_mint.key(),
                &ctx.accounts.config.usdc_mint,
                referrer,
                ctx.remaining_accounts[start].clone(),
                ctx.remaining_accounts[start + 1].clone(),
                ctx.remaining_accounts[start + 2].clone(),
                &ctx.remaining_accounts[start + 3],
            )?;

            used_accounts = end;
            expected_referrer = next_referrer;

            let ReferralNode {
                reward_vault_info,
                reward_usdc_info,
                reward_vault,
            } = node;

            let share = usdc_price
                .checked_mul(REFERRAL_BPS[depth])
                .ok_or(ErrorCode::MathOverflow)?
                .checked_div(BASIS_POINTS)
                .ok_or(ErrorCode::MathOverflow)?;
            if share == 0 {
                continue;
            }

            token::transfer(
                CpiContext::new(
                    token_program_info.clone(),
                    Transfer {
                        from: buyer_usdc_info.clone(),
                        to: reward_usdc_info.clone(),
                        authority: buyer_info.clone(),
                    },
                ),
                share,
            )?;

            let mut updated_vault = reward_vault;
            updated_vault.claimable_usdc = updated_vault
                .claimable_usdc
                .checked_add(share)
                .ok_or(ErrorCode::MathOverflow)?;
            distributed = distributed
                .checked_add(share)
                .ok_or(ErrorCode::MathOverflow)?;

            {
                let mut data = reward_vault_info.try_borrow_mut_data()?;
                let mut slice: &mut [u8] = &mut data;
                updated_vault.try_serialize(&mut slice)?;
            }
        }

        if used_accounts != ctx.remaining_accounts.len() {
            return err!(ErrorCode::TooManyReferralAccounts);
        }

        let remaining = usdc_price
            .checked_sub(distributed)
            .ok_or(ErrorCode::MathOverflow)?;
        if remaining > 0 {
            token::transfer(
                CpiContext::new(
                    token_program_info,
                    Transfer {
                        from: buyer_usdc_info,
                        to: treasury_usdc_info,
                        authority: buyer_info,
                    },
                ),
                remaining,
            )?;
        }

        mint_collection_nft(
            ctx.accounts.config.bump,
            ctx.accounts.collection_mint.to_account_info(),
            ctx.accounts.buyer_nft_account.to_account_info(),
            ctx.accounts.config.to_account_info(),
            ctx.accounts.token_program.to_account_info(),
        )?;

        let order = &mut ctx.accounts.order;
        order.used = true;
        order.seed = order_id
            .as_slice()
            .try_into()
            .map_err(|_| ErrorCode::InvalidOrderId)?;
        order.buyer = buyer_key;
        order.bump =
            Pubkey::find_program_address(&[ORDER_SEED, order.seed.as_ref()], ctx.program_id).1;

        Ok(())
    }

    pub fn claim_sol(ctx: Context<ClaimSol>) -> Result<()> {
        let reward_vault = &mut ctx.accounts.reward_vault;
        let amount = reward_vault.claimable_sol;
        require!(amount > 0, ErrorCode::NothingToClaim);

        let vault_info = reward_vault.to_account_info();
        require!(
            vault_info.lamports() >= amount,
            ErrorCode::InsufficientVaultFunds
        );

        **vault_info.try_borrow_mut_lamports()? -= amount;
        **ctx
            .accounts
            .user
            .to_account_info()
            .try_borrow_mut_lamports()? += amount;

        reward_vault.claimable_sol = 0;

        Ok(())
    }

    pub fn claim_usdc(ctx: Context<ClaimUsdc>) -> Result<()> {
        let reward_vault = &mut ctx.accounts.reward_vault;
        let amount = reward_vault.claimable_usdc;
        require!(amount > 0, ErrorCode::NothingToClaim);

        let seeds = &[
            REWARD_SEED,
            reward_vault.user.as_ref(),
            &[reward_vault.bump],
        ];
        let signer_seeds = &[&seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.reward_usdc.to_account_info(),
                    to: ctx.accounts.user_usdc.to_account_info(),
                    authority: reward_vault.to_account_info(),
                },
                signer_seeds,
            ),
            amount,
        )?;

        reward_vault.claimable_usdc = 0;

        Ok(())
    }
}

fn mint_collection_nft<'info>(
    config_bump: u8,
    mint: AccountInfo<'info>,
    destination: AccountInfo<'info>,
    config_info: AccountInfo<'info>,
    token_program: AccountInfo<'info>,
) -> Result<()> {
    let seeds = &[CONFIG_SEED, &[config_bump]];
    let signer = &[&seeds[..]];
    token::mint_to(
        CpiContext::new_with_signer(
            token_program,
            MintTo {
                mint,
                to: destination,
                authority: config_info,
            },
            signer,
        ),
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
    /// CHECK: stored as provided
    #[account(mut)]
    pub treasury_sol: AccountInfo<'info>,
    #[account(mut)]
    pub treasury_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub collection_mint: Account<'info, Mint>,
    pub system_program: Program<'info, System>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitializeArgs {
    pub admin: Pubkey,
    pub usdc_mint: Pubkey,
    pub collection_mint: Pubkey,
    pub sol_price: u64,
    pub usdc_price: u64,
}

#[derive(Accounts)]
pub struct RegisterUser<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        init,
        payer = payer,
        space = ReferralProfile::LEN,
        seeds = [PROFILE_SEED, user.key().as_ref()],
        bump
    )]
    pub profile: Account<'info, ReferralProfile>,
    #[account(
        init,
        payer = payer,
        space = RewardVault::LEN,
        seeds = [REWARD_SEED, user.key().as_ref()],
        bump
    )]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(address = config.usdc_mint)]
    pub usdc_mint: Account<'info, Mint>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = usdc_mint,
        associated_token::authority = reward_vault
    )]
    pub reward_usdc: Account<'info, TokenAccount>,
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SetReferrer<'info> {
    #[account(mut, seeds = [PROFILE_SEED, user.key().as_ref()], bump = profile.bump)]
    pub profile: Account<'info, ReferralProfile>,
    pub user: Signer<'info>,
    #[account(seeds = [PROFILE_SEED, parent_profile.user.as_ref()], bump = parent_profile.bump)]
    pub parent_profile: Account<'info, ReferralProfile>,
}

#[derive(Accounts)]
#[instruction(order_id: Vec<u8>)]
pub struct MintWithSol<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    /// CHECK: validated via config
    #[account(mut, address = config.treasury_sol)]
    pub treasury_sol: AccountInfo<'info>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(
        init_if_needed,
        payer = buyer,
        space = Order::LEN,
        seeds = [ORDER_SEED, order_id.as_slice()],
        bump
    )]
    pub order: Account<'info, Order>,
    #[account(mut, seeds = [PROFILE_SEED, buyer.key().as_ref()], bump = buyer_profile.bump)]
    pub buyer_profile: Account<'info, ReferralProfile>,
    #[account(mut, seeds = [REWARD_SEED, buyer.key().as_ref()], bump = buyer_reward_vault.bump)]
    pub buyer_reward_vault: Account<'info, RewardVault>,
    #[account(mut, address = config.collection_mint)]
    pub collection_mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = buyer_nft_account.owner == buyer.key(),
        constraint = buyer_nft_account.mint == collection_mint.key()
    )]
    pub buyer_nft_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(order_id: Vec<u8>)]
pub struct MintWithUsdc<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump)]
    pub config: Account<'info, Config>,
    #[account(mut, address = config.treasury_sol)]
    /// CHECK: not used but kept for parity
    pub treasury_sol: AccountInfo<'info>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(
        init_if_needed,
        payer = buyer,
        space = Order::LEN,
        seeds = [ORDER_SEED, order_id.as_slice()],
        bump
    )]
    pub order: Account<'info, Order>,
    #[account(mut, seeds = [PROFILE_SEED, buyer.key().as_ref()], bump = buyer_profile.bump)]
    pub buyer_profile: Account<'info, ReferralProfile>,
    #[account(mut, seeds = [REWARD_SEED, buyer.key().as_ref()], bump = buyer_reward_vault.bump)]
    pub buyer_reward_vault: Account<'info, RewardVault>,
    #[account(mut, address = config.collection_mint)]
    pub collection_mint: Account<'info, Mint>,
    #[account(
        mut,
        constraint = buyer_nft_account.owner == buyer.key(),
        constraint = buyer_nft_account.mint == collection_mint.key()
    )]
    pub buyer_nft_account: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = buyer_usdc.owner == buyer.key(),
        constraint = buyer_usdc.mint == config.usdc_mint
    )]
    pub buyer_usdc: Account<'info, TokenAccount>,
    #[account(
        mut,
        address = config.treasury_usdc,
        constraint = treasury_usdc.mint == config.usdc_mint
    )]
    pub treasury_usdc: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimSol<'info> {
    #[account(mut, seeds = [REWARD_SEED, user.key().as_ref()], bump = reward_vault.bump)]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimUsdc<'info> {
    #[account(mut, seeds = [REWARD_SEED, user.key().as_ref()], bump = reward_vault.bump)]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(
        mut,
        constraint = reward_usdc.owner == reward_vault.key()
    )]
    pub reward_usdc: Account<'info, TokenAccount>,
    #[account(
        mut,
        constraint = user_usdc.owner == user.key(),
        constraint = user_usdc.mint == reward_usdc.mint
    )]
    pub user_usdc: Account<'info, TokenAccount>,
    #[account(mut, address = reward_vault.user)]
    pub user: Signer<'info>,
    pub token_program: Program<'info, Token>,
}

fn validate_referral<'info>(
    program_id: &Pubkey,
    collection_mint: &Pubkey,
    usdc_mint: &Pubkey,
    referrer: Pubkey,
    profile_info: AccountInfo<'info>,
    reward_vault_info: AccountInfo<'info>,
    reward_usdc_info: AccountInfo<'info>,
    nft_info: &AccountInfo<'info>,
) -> Result<(ReferralNode<'info>, Option<Pubkey>)> {
    let derived_profile =
        Pubkey::find_program_address(&[PROFILE_SEED, referrer.as_ref()], program_id).0;
    require_keys_eq!(
        derived_profile,
        profile_info.key(),
        ErrorCode::InvalidReferralAccount
    );

    let derived_reward =
        Pubkey::find_program_address(&[REWARD_SEED, referrer.as_ref()], program_id).0;
    require_keys_eq!(
        derived_reward,
        reward_vault_info.key(),
        ErrorCode::InvalidReferralAccount
    );

    let expected_nft = get_associated_token_address(&referrer, collection_mint);
    require_keys_eq!(
        expected_nft,
        nft_info.key(),
        ErrorCode::InvalidReferralAccount
    );

    let profile = {
        let data = profile_info.try_borrow_data()?;
        let mut slice: &[u8] = &data;
        let account: ReferralProfile = ReferralProfile::try_deserialize(&mut slice)?;
        require_keys_eq!(account.user, referrer, ErrorCode::InvalidReferralAccount);
        account
    };

    let reward_vault = {
        let data = reward_vault_info.try_borrow_data()?;
        let mut slice: &[u8] = &data;
        let account: RewardVault = RewardVault::try_deserialize(&mut slice)?;
        require_keys_eq!(account.user, referrer, ErrorCode::InvalidReferralAccount);
        account
    };

    let _reward_usdc = {
        let data = reward_usdc_info.try_borrow_data()?;
        let mut slice: &[u8] = &data;
        let account: TokenAccount = TokenAccount::try_deserialize(&mut slice)?;
        require_keys_eq!(
            account.owner,
            reward_vault_info.key(),
            ErrorCode::InvalidReferralAccount
        );
        require_keys_eq!(account.mint, *usdc_mint, ErrorCode::InvalidReferralAccount);
        account
    };

    let next_referrer = if profile.has_referrer {
        Some(profile.referrer)
    } else {
        None
    };

    Ok((
        ReferralNode {
            reward_vault_info,
            reward_usdc_info,
            reward_vault,
        },
        next_referrer,
    ))
}

struct ReferralNode<'info> {
    reward_vault_info: AccountInfo<'info>,
    reward_usdc_info: AccountInfo<'info>,
    reward_vault: RewardVault,
}

#[account]
pub struct Config {
    pub admin: Pubkey,
    pub treasury_sol: Pubkey,
    pub treasury_usdc: Pubkey,
    pub collection_mint: Pubkey,
    pub usdc_mint: Pubkey,
    pub sol_price: u64,
    pub usdc_price: u64,
    pub bump: u8,
}

impl Config {
    pub const LEN: usize = 8 + (32 * 5) + 8 + 8 + 1;
}

#[account]
pub struct ReferralProfile {
    pub user: Pubkey,
    pub has_referrer: bool,
    pub referrer: Pubkey,
    pub bump: u8,
}

impl ReferralProfile {
    pub const LEN: usize = 8 + 32 + 1 + 32 + 1;
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
    pub seed: [u8; 16],
    pub buyer: Pubkey,
    pub used: bool,
    pub bump: u8,
}

impl Order {
    pub const LEN: usize = 8 + 16 + 32 + 1 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("The provided admin is invalid")]
    InvalidAdmin,
    #[msg("The provided price is invalid")]
    InvalidPrice,
    #[msg("Referrer already set for this profile")]
    ReferrerAlreadySet,
    #[msg("Invalid referrer provided")]
    InvalidReferrer,
    #[msg("Order identifier is invalid")]
    InvalidOrderId,
    #[msg("This order has already been used")]
    OrderAlreadyUsed,
    #[msg("Profile mismatch")]
    ProfileMismatch,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Missing referral accounts")]
    MissingReferralAccounts,
    #[msg("Too many referral accounts provided")]
    TooManyReferralAccounts,
    #[msg("Referral account validation failed")]
    InvalidReferralAccount,
    #[msg("Nothing to claim")]
    NothingToClaim,
    #[msg("Insufficient funds in reward vault")]
    InsufficientVaultFunds,
    #[msg("Collection mint does not match the provided configuration")]
    InvalidCollectionMint,
    #[msg("Treasury account configuration is invalid")]
    InvalidTreasuryAccount,
}
