use anchor_lang::prelude::*;
use anchor_lang::solana_program::{program::invoke, program::invoke_signed, system_instruction};
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

pub const CONFIG_SEED: &[u8] = b"config";
pub const PROFILE_SEED: &[u8] = b"profile";
pub const REWARD_VAULT_SEED: &[u8] = b"reward";
pub const ORDER_SEED: &[u8] = b"order";

/// Program id dedicated for this open-source repository.
declare_id!("Trsa1esReferr4l1ju8GhhQXUfdViQspuWqX9u9KQk8k");

#[program]
pub mod trsales_license {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, args: InitializeArgs) -> Result<()> {
        require!(args.sol_price > 0, ErrorCode::InvalidPrice);
        require!(args.usdc_price > 0, ErrorCode::InvalidPrice);

        let config = &mut ctx.accounts.config;
        config.admin = args.admin;
        config.usdc_mint = args.usdc_mint;
        config.collection_mint = args.collection_mint;
        config.sol_price = args.sol_price;
        config.usdc_price = args.usdc_price;
        config.bump = *ctx.bumps.get("config").ok_or(ErrorCode::BumpNotFound)?;
        config.treasury_sol = ctx.accounts.treasury_sol.key();
        config.treasury_usdc = ctx.accounts.treasury_usdc.key();
        config.paused = false;

        require_keys_eq!(
            ctx.accounts.collection_mint.mint_authority.unwrap(),
            config.key(),
            ErrorCode::InvalidMintAuthority,
        );
        require_keys_eq!(
            ctx.accounts.collection_mint.freeze_authority.unwrap(),
            config.key(),
            ErrorCode::InvalidMintAuthority,
        );

        Ok(())
    }

    pub fn update_config(ctx: Context<UpdateConfig>, args: UpdateConfigArgs) -> Result<()> {
        require!(args.sol_price > 0, ErrorCode::InvalidPrice);
        require!(args.usdc_price > 0, ErrorCode::InvalidPrice);
        let config = &mut ctx.accounts.config;
        config.sol_price = args.sol_price;
        config.usdc_price = args.usdc_price;
        config.paused = args.paused;
        if let Some(mint) = args.collection_mint {
            config.collection_mint = mint;
        }
        if let Some(usdc) = args.usdc_mint {
            config.usdc_mint = usdc;
        }
        Ok(())
    }

    pub fn register_user(ctx: Context<RegisterUser>) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        profile.user = ctx.accounts.user.key();
        profile.has_referrer = false;
        profile.referrer = Pubkey::default();
        profile.bump = *ctx.bumps.get("profile").ok_or(ErrorCode::BumpNotFound)?;

        let reward_vault = &mut ctx.accounts.reward_vault;
        reward_vault.user = profile.user;
        reward_vault.bump = *ctx
            .bumps
            .get("reward_vault")
            .ok_or(ErrorCode::BumpNotFound)?;
        reward_vault.claimable_sol = 0;
        reward_vault.claimable_usdc = 0;
        reward_vault.total_claimed_sol = 0;
        reward_vault.total_claimed_usdc = 0;
        reward_vault.total_earned_sol = 0;
        reward_vault.total_earned_usdc = 0;

        Ok(())
    }

    pub fn set_referrer(ctx: Context<SetReferrer>, parent: Pubkey) -> Result<()> {
        let profile = &mut ctx.accounts.profile;
        require!(!profile.has_referrer, ErrorCode::ReferrerAlreadySet);
        require_keys_ne!(profile.user, parent, ErrorCode::SelfReferral);
        let parent_profile = &ctx.accounts.parent_profile;
        require_keys_eq!(
            parent_profile.user,
            parent,
            ErrorCode::MismatchedParentProfile
        );

        if parent_profile.has_referrer {
            require_ne!(
                parent_profile.referrer,
                profile.user,
                ErrorCode::ReferralLoopDetected
            );
        }

        profile.has_referrer = true;
        profile.referrer = parent;
        Ok(())
    }

    pub fn mint_with_sol(ctx: Context<MintWithSol>, order_id: [u8; 16]) -> Result<()> {
        let config = &ctx.accounts.config;
        require!(!config.paused, ErrorCode::MintingPaused);
        let price = config.sol_price;
        require!(price > 0, ErrorCode::InvalidPrice);

        let (mut order_account, _) = init_order_receipt(
            &ctx.accounts.order,
            &ctx.accounts.buyer,
            &ctx.accounts.system_program,
            &order_id,
        )?;

        process_mint(
            MintContext {
                config,
                order: &mut order_account,
                buyer: &ctx.accounts.buyer,
                buyer_profile: &ctx.accounts.buyer_profile,
                buyer_reward_vault: &mut ctx.accounts.buyer_reward_vault,
                collection_mint: &ctx.accounts.collection_mint,
                buyer_nft_account: &ctx.accounts.buyer_nft_account,
                token_program: &ctx.accounts.token_program,
                system_program: &ctx.accounts.system_program,
                treasury_sol: &ctx.accounts.treasury_sol,
                treasury_usdc: None,
                buyer_usdc: None,
                remaining_accounts: ctx.remaining_accounts,
            },
            Currency::Sol,
            price,
            order_id,
        )?;

        order_account.exit()?;
        Ok(())
    }

    pub fn mint_with_usdc(ctx: Context<MintWithUsdc>, order_id: [u8; 16]) -> Result<()> {
        let config = &ctx.accounts.config;
        require!(!config.paused, ErrorCode::MintingPaused);
        let price = config.usdc_price;
        require!(price > 0, ErrorCode::InvalidPrice);

        let (mut order_account, _) = init_order_receipt(
            &ctx.accounts.order,
            &ctx.accounts.buyer,
            &ctx.accounts.system_program,
            &order_id,
        )?;

        process_mint(
            MintContext {
                config,
                order: &mut order_account,
                buyer: &ctx.accounts.buyer,
                buyer_profile: &ctx.accounts.buyer_profile,
                buyer_reward_vault: &mut ctx.accounts.buyer_reward_vault,
                collection_mint: &ctx.accounts.collection_mint,
                buyer_nft_account: &ctx.accounts.buyer_nft_account,
                token_program: &ctx.accounts.token_program,
                system_program: &ctx.accounts.system_program,
                treasury_sol: &ctx.accounts.treasury_sol,
                treasury_usdc: Some(&ctx.accounts.treasury_usdc),
                buyer_usdc: Some(&ctx.accounts.buyer_usdc),
                remaining_accounts: ctx.remaining_accounts,
            },
            Currency::Usdc,
            price,
            order_id,
        )?;

        order_account.exit()?;
        Ok(())
    }

    pub fn claim_sol(ctx: Context<ClaimSol>) -> Result<()> {
        let vault = &mut ctx.accounts.reward_vault;
        let amount = vault.claimable_sol;
        require!(amount > 0, ErrorCode::NoRewardsToClaim);
        vault.claimable_sol = 0;
        vault.total_claimed_sol = vault
            .total_claimed_sol
            .checked_add(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let seeds = &[REWARD_VAULT_SEED, vault.user.as_ref(), &[vault.bump]];
        let signer = &[&seeds[..]];
        invoke_signed(
            &system_instruction::transfer(&vault.key(), &ctx.accounts.user.key(), amount),
            &[
                vault.to_account_info(),
                ctx.accounts.user.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
            ],
            signer,
        )?;
        Ok(())
    }

    pub fn claim_usdc(ctx: Context<ClaimUsdc>) -> Result<()> {
        let vault = &mut ctx.accounts.reward_vault;
        let amount = vault.claimable_usdc;
        require!(amount > 0, ErrorCode::NoRewardsToClaim);
        vault.claimable_usdc = 0;
        vault.total_claimed_usdc = vault
            .total_claimed_usdc
            .checked_add(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let seeds = &[REWARD_VAULT_SEED, vault.user.as_ref(), &[vault.bump]];
        let signer = &[&seeds[..]];
        let cpi_accounts = Transfer {
            from: ctx.accounts.reward_usdc.to_account_info(),
            to: ctx.accounts.user_usdc.to_account_info(),
            authority: vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            cpi_accounts,
            signer,
        );
        token::transfer(cpi_ctx, amount)?;
        Ok(())
    }
}

fn init_order_receipt<'info>(
    order_info: &AccountInfo<'info>,
    payer: &Signer<'info>,
    system_program: &Program<'info, System>,
    order_id: &[u8; 16],
) -> Result<(Account<'info, OrderReceipt>, u8)> {
    let (expected, bump) =
        Pubkey::find_program_address(&[ORDER_SEED, order_id.as_ref()], &crate::ID);
    require_keys_eq!(order_info.key(), expected, ErrorCode::InvalidOrderAccount);
    if order_info.data_is_empty() {
        let rent = Rent::get()?;
        let lamports = rent.minimum_balance(8 + OrderReceipt::LEN);
        let create_ix = system_instruction::create_account(
            &payer.key(),
            &expected,
            lamports,
            (8 + OrderReceipt::LEN) as u64,
            &crate::ID,
        );
        invoke_signed(
            &create_ix,
            &[
                payer.to_account_info(),
                order_info.clone(),
                system_program.to_account_info(),
            ],
            &[&[ORDER_SEED, order_id.as_ref(), &[bump]]],
        )?;
    }
    let account = Account::<OrderReceipt>::try_from(order_info)?;
    Ok((account, bump))
}

struct MintContext<'info> {
    pub config: &'info Account<'info, Config>,
    pub order: &'info mut Account<'info, OrderReceipt>,
    pub buyer: &'info Signer<'info>,
    pub buyer_profile: &'info Account<'info, ReferralProfile>,
    pub buyer_reward_vault: &'info mut Account<'info, RewardVault>,
    pub collection_mint: &'info Account<'info, Mint>,
    pub buyer_nft_account: &'info Account<'info, TokenAccount>,
    pub token_program: &'info Program<'info, Token>,
    pub system_program: &'info Program<'info, System>,
    pub treasury_sol: &'info AccountInfo<'info>,
    pub treasury_usdc: Option<&'info Account<'info, TokenAccount>>,
    pub buyer_usdc: Option<&'info Account<'info, TokenAccount>>,
    pub remaining_accounts: &'info [AccountInfo<'info>],
}

fn process_mint<'info>(
    mut ctx: MintContext<'info>,
    currency: Currency,
    price: u64,
    order_id: [u8; 16],
) -> Result<()> {
    require_keys_eq!(
        ctx.buyer_profile.user,
        ctx.buyer.key(),
        ErrorCode::UnauthorizedProfile,
    );
    require_keys_eq!(
        ctx.buyer_reward_vault.user,
        ctx.buyer.key(),
        ErrorCode::UnauthorizedRewardVault,
    );
    require_keys_eq!(
        ctx.collection_mint.key(),
        ctx.config.collection_mint,
        ErrorCode::InvalidCollectionMint,
    );
    require_keys_eq!(
        ctx.buyer_nft_account.mint,
        ctx.config.collection_mint,
        ErrorCode::InvalidCollectionMint,
    );
    require_keys_eq!(
        ctx.buyer_nft_account.owner,
        ctx.buyer.key(),
        ErrorCode::InvalidNftOwner,
    );

    ctx.order.order_id = order_id;
    ctx.order.payer = ctx.buyer.key();
    ctx.order.amount = price;
    ctx.order.currency = currency.into();

    let mut accounts_iter = ctx.remaining_accounts.iter();
    let mut next_ref = if ctx.buyer_profile.has_referrer {
        Some(ctx.buyer_profile.referrer)
    } else {
        None
    };
    let mut resolved: Vec<ResolvedReferral<'info>> = Vec::with_capacity(3);

    for _ in 0..3 {
        if let Some(ref_key) = next_ref {
            let profile_info = match accounts_iter.next() {
                Some(acc) => acc,
                None => break,
            };
            let profile = Account::<ReferralProfile>::try_from(profile_info)?;
            if profile.user != ref_key {
                break;
            }
            let vault_info = match accounts_iter.next() {
                Some(acc) => acc,
                None => break,
            };
            let reward_vault = Account::<RewardVault>::try_from(vault_info)?;
            require_keys_eq!(
                reward_vault.user,
                ref_key,
                ErrorCode::UnauthorizedRewardVault
            );
            let reward_usdc_info = match accounts_iter.next() {
                Some(acc) => acc,
                None => break,
            };
            let reward_usdc = Account::<TokenAccount>::try_from(reward_usdc_info)?;
            require_keys_eq!(
                reward_usdc.owner,
                reward_vault.key(),
                ErrorCode::InvalidTokenAccount
            );
            require_keys_eq!(
                reward_usdc.mint,
                ctx.config.usdc_mint,
                ErrorCode::InvalidTokenAccount
            );
            let nft_info = match accounts_iter.next() {
                Some(acc) => acc,
                None => break,
            };
            let nft_account = Account::<TokenAccount>::try_from(nft_info)?;
            require_keys_eq!(
                nft_account.mint,
                ctx.config.collection_mint,
                ErrorCode::InvalidCollectionMint
            );
            require_keys_eq!(nft_account.owner, ref_key, ErrorCode::InvalidNftOwner);

            let has_next = profile.has_referrer.then_some(profile.referrer);
            resolved.push(ResolvedReferral {
                profile,
                reward_vault,
                reward_usdc,
                nft_account,
            });
            next_ref = has_next;
        }
    }

    let shares = ReferralShares::new(price)?;

    match currency {
        Currency::Sol => {
            for (idx, info) in resolved.iter_mut().enumerate() {
                let amount = shares.share(idx);
                if amount == 0 {
                    continue;
                }
                if info.nft_account.amount == 0 {
                    transfer_sol_from_buyer(
                        ctx.buyer,
                        ctx.treasury_sol,
                        ctx.system_program,
                        amount,
                    )?;
                    continue;
                }
                transfer_sol_from_buyer(
                    ctx.buyer,
                    &info.reward_vault.to_account_info(),
                    ctx.system_program,
                    amount,
                )?;
                info.reward_vault.claimable_sol = info
                    .reward_vault
                    .claimable_sol
                    .checked_add(amount)
                    .ok_or(ErrorCode::MathOverflow)?;
                info.reward_vault.total_earned_sol = info
                    .reward_vault
                    .total_earned_sol
                    .checked_add(amount)
                    .ok_or(ErrorCode::MathOverflow)?;
            }
            for idx in resolved.len()..3 {
                let amount = shares.share(idx);
                if amount > 0 {
                    transfer_sol_from_buyer(
                        ctx.buyer,
                        ctx.treasury_sol,
                        ctx.system_program,
                        amount,
                    )?;
                }
            }
        }
        Currency::Usdc => {
            let buyer_usdc = ctx.buyer_usdc.ok_or(ErrorCode::MissingBuyerTokenAccount)?;
            require_keys_eq!(
                buyer_usdc.owner,
                ctx.buyer.key(),
                ErrorCode::InvalidTokenAccount
            );
            require_keys_eq!(
                buyer_usdc.mint,
                ctx.config.usdc_mint,
                ErrorCode::InvalidTokenAccount
            );
            let treasury_usdc = ctx.treasury_usdc.ok_or(ErrorCode::InvalidTokenAccount)?;

            for (idx, info) in resolved.iter_mut().enumerate() {
                let amount = shares.share(idx);
                if amount == 0 {
                    continue;
                }
                if info.nft_account.amount == 0 {
                    transfer_usdc_from_buyer(
                        ctx.token_program,
                        buyer_usdc,
                        treasury_usdc,
                        ctx.buyer,
                        amount,
                    )?;
                    continue;
                }
                transfer_usdc_from_buyer(
                    ctx.token_program,
                    buyer_usdc,
                    &info.reward_usdc,
                    ctx.buyer,
                    amount,
                )?;
                info.reward_vault.claimable_usdc = info
                    .reward_vault
                    .claimable_usdc
                    .checked_add(amount)
                    .ok_or(ErrorCode::MathOverflow)?;
                info.reward_vault.total_earned_usdc = info
                    .reward_vault
                    .total_earned_usdc
                    .checked_add(amount)
                    .ok_or(ErrorCode::MathOverflow)?;
            }
            for idx in resolved.len()..3 {
                let amount = shares.share(idx);
                if amount > 0 {
                    transfer_usdc_from_buyer(
                        ctx.token_program,
                        buyer_usdc,
                        treasury_usdc,
                        ctx.buyer,
                        amount,
                    )?;
                }
            }
        }
    }

    for info in resolved.iter_mut() {
        info.reward_vault.exit()?;
    }

    let seeds = &[CONFIG_SEED, &[ctx.config.bump]];
    let signer = &[&seeds[..]];
    let cpi_accounts = MintTo {
        mint: ctx.collection_mint.to_account_info(),
        to: ctx.buyer_nft_account.to_account_info(),
        authority: ctx.config.to_account_info(),
    };
    let cpi_ctx =
        CpiContext::new_with_signer(ctx.token_program.to_account_info(), cpi_accounts, signer);
    token::mint_to(cpi_ctx, 1)?;

    Ok(())
}

fn transfer_sol_from_buyer(
    buyer: &Signer,
    destination: &AccountInfo,
    system_program: &Program<System>,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    invoke(
        &system_instruction::transfer(&buyer.key(), destination.key, amount),
        &[
            buyer.to_account_info(),
            destination.clone(),
            system_program.to_account_info(),
        ],
    )?;
    Ok(())
}

fn transfer_usdc_from_buyer(
    token_program: &Program<Token>,
    from: &Account<TokenAccount>,
    to: &Account<TokenAccount>,
    authority: &Signer,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let cpi_accounts = Transfer {
        from: from.to_account_info(),
        to: to.to_account_info(),
        authority: authority.to_account_info(),
    };
    let cpi_ctx = CpiContext::new(token_program.to_account_info(), cpi_accounts);
    token::transfer(cpi_ctx, amount)?;
    Ok(())
}

struct ResolvedReferral<'info> {
    profile: Account<'info, ReferralProfile>,
    reward_vault: Account<'info, RewardVault>,
    reward_usdc: Account<'info, TokenAccount>,
    nft_account: Account<'info, TokenAccount>,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct InitializeArgs {
    pub admin: Pubkey,
    pub usdc_mint: Pubkey,
    pub collection_mint: Pubkey,
    pub sol_price: u64,
    pub usdc_price: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct UpdateConfigArgs {
    pub sol_price: u64,
    pub usdc_price: u64,
    pub paused: bool,
    pub collection_mint: Option<Pubkey>,
    pub usdc_mint: Option<Pubkey>,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(
        init,
        payer = payer,
        space = 8 + Config::LEN,
        seeds = [CONFIG_SEED],
        bump
    )]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut)]
    pub treasury_sol: AccountInfo<'info>,
    #[account(mut)]
    pub treasury_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub collection_mint: Account<'info, Mint>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = admin)]
    pub config: Account<'info, Config>,
    pub admin: Signer<'info>,
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
        space = 8 + ReferralProfile::LEN,
        seeds = [PROFILE_SEED, user.key().as_ref()],
        bump
    )]
    pub profile: Account<'info, ReferralProfile>,
    #[account(
        init,
        payer = payer,
        space = 8 + RewardVault::LEN,
        seeds = [REWARD_VAULT_SEED, user.key().as_ref()],
        bump,
    )]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(
        init,
        payer = payer,
        associated_token::mint = config.usdc_mint,
        associated_token::authority = reward_vault
    )]
    pub reward_usdc: Account<'info, TokenAccount>,
    /// CHECK: wallet being registered
    pub user: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SetReferrer<'info> {
    #[account(mut, seeds = [PROFILE_SEED, user.key().as_ref()], bump = profile.bump, has_one = user)]
    pub profile: Account<'info, ReferralProfile>,
    pub user: Signer<'info>,
    #[account(seeds = [PROFILE_SEED, parent.key().as_ref()], bump = parent_profile.bump)]
    pub parent_profile: Account<'info, ReferralProfile>,
}

#[derive(Accounts)]
pub struct MintWithSol<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = treasury_sol)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub treasury_sol: AccountInfo<'info>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    /// CHECK: PDA derived from order id
    #[account(mut)]
    pub order: UncheckedAccount<'info>,
    #[account(mut, seeds = [PROFILE_SEED, buyer.key().as_ref()], bump = buyer_profile.bump)]
    pub buyer_profile: Account<'info, ReferralProfile>,
    #[account(mut, seeds = [REWARD_VAULT_SEED, buyer.key().as_ref()], bump = buyer_reward_vault.bump)]
    pub buyer_reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub collection_mint: Account<'info, Mint>,
    #[account(mut)]
    pub buyer_nft_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct MintWithUsdc<'info> {
    #[account(mut, seeds = [CONFIG_SEED], bump = config.bump, has_one = treasury_sol, has_one = treasury_usdc)]
    pub config: Account<'info, Config>,
    #[account(mut)]
    pub treasury_sol: AccountInfo<'info>,
    #[account(mut)]
    pub treasury_usdc: Account<'info, TokenAccount>,
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(mut)]
    pub buyer_usdc: Account<'info, TokenAccount>,
    /// CHECK: PDA derived from order id
    #[account(mut)]
    pub order: UncheckedAccount<'info>,
    #[account(mut, seeds = [PROFILE_SEED, buyer.key().as_ref()], bump = buyer_profile.bump)]
    pub buyer_profile: Account<'info, ReferralProfile>,
    #[account(mut, seeds = [REWARD_VAULT_SEED, buyer.key().as_ref()], bump = buyer_reward_vault.bump)]
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
    #[account(mut, seeds = [REWARD_VAULT_SEED, user.key().as_ref()], bump = reward_vault.bump, has_one = user)]
    pub reward_vault: Account<'info, RewardVault>,
    #[account(mut)]
    pub user: Signer<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ClaimUsdc<'info> {
    #[account(mut, seeds = [REWARD_VAULT_SEED, user.key().as_ref()], bump = reward_vault.bump, has_one = user)]
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
    pub treasury_sol: Pubkey,
    pub treasury_usdc: Pubkey,
    pub usdc_mint: Pubkey,
    pub collection_mint: Pubkey,
    pub sol_price: u64,
    pub usdc_price: u64,
    pub bump: u8,
    pub paused: bool,
}

impl Config {
    pub const LEN: usize = 32 * 5 + 8 * 2 + 1 + 1;
}

#[account]
pub struct ReferralProfile {
    pub user: Pubkey,
    pub has_referrer: bool,
    pub referrer: Pubkey,
    pub bump: u8,
}

impl ReferralProfile {
    pub const LEN: usize = 32 + 1 + 32 + 1;
}

#[account]
pub struct RewardVault {
    pub user: Pubkey,
    pub claimable_sol: u64,
    pub claimable_usdc: u64,
    pub total_claimed_sol: u64,
    pub total_claimed_usdc: u64,
    pub total_earned_sol: u64,
    pub total_earned_usdc: u64,
    pub bump: u8,
}

impl RewardVault {
    pub const LEN: usize = 32 + 8 * 6 + 1;
}

#[account]
pub struct OrderReceipt {
    pub order_id: [u8; 16],
    pub payer: Pubkey,
    pub amount: u64,
    pub currency: u8,
}

impl OrderReceipt {
    pub const LEN: usize = 16 + 32 + 8 + 1;
}

#[derive(Clone, Copy)]
pub enum Currency {
    Sol,
    Usdc,
}

impl Currency {
    fn into(self) -> u8 {
        match self {
            Currency::Sol => 0,
            Currency::Usdc => 1,
        }
    }
}

struct ReferralShares {
    l1: u64,
    l2: u64,
    l3: u64,
}

impl ReferralShares {
    fn new(total: u64) -> Result<Self> {
        let l1 = total.checked_mul(50).ok_or(ErrorCode::MathOverflow)? / 100;
        let l2 = total.checked_mul(30).ok_or(ErrorCode::MathOverflow)? / 100;
        let l3 = total.checked_mul(20).ok_or(ErrorCode::MathOverflow)? / 100;
        Ok(Self { l1, l2, l3 })
    }

    fn share(&self, idx: usize) -> u64 {
        match idx {
            0 => self.l1,
            1 => self.l2,
            2 => self.l3,
            _ => 0,
        }
    }
}

#[error_code]
pub enum ErrorCode {
    #[msg("Bump not found")]
    BumpNotFound,
    #[msg("Invalid price configuration")]
    InvalidPrice,
    #[msg("Mint authority mismatch")]
    InvalidMintAuthority,
    #[msg("Minting is paused")]
    MintingPaused,
    #[msg("Profile is not for this user")]
    UnauthorizedProfile,
    #[msg("Reward vault is not for this user")]
    UnauthorizedRewardVault,
    #[msg("Invalid collection mint provided")]
    InvalidCollectionMint,
    #[msg("Invalid NFT owner")]
    InvalidNftOwner,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("No rewards available to claim")]
    NoRewardsToClaim,
    #[msg("Self-referral is forbidden")]
    SelfReferral,
    #[msg("Missing parent profile")]
    MissingParentProfile,
    #[msg("Parent profile mismatch")]
    MismatchedParentProfile,
    #[msg("Referrer already configured")]
    ReferrerAlreadySet,
    #[msg("Referral loop detected")]
    ReferralLoopDetected,
    #[msg("Missing buyer token account")]
    MissingBuyerTokenAccount,
    #[msg("Invalid token account")]
    InvalidTokenAccount,
    #[msg("Invalid order account")]
    InvalidOrderAccount,
}
