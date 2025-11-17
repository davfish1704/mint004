import * as anchor from "@project-serum/anchor";
import { Program } from "@project-serum/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  createMint,
  mintTo,
} from "@solana/spl-token";
import { Keypair, SystemProgram, PublicKey, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import idl from "../target/idl/trsales_license.json";
import { TrsalesLicense } from "../target/types/trsales_license";

const provider = anchor.AnchorProvider.env();
anchor.setProvider(provider);

if (!provider || !provider.wallet || !provider.wallet.publicKey) {
  throw new Error("Provider wallet or wallet.publicKey is undefined. Make sure AnchorProvider.env() is configured with a wallet.");
}

const PROGRAM_ID = new PublicKey("Trsa1esReferr4l1ju8GhhQXUfdViQspuWqX9u9KQk8k");
const workspace: any = anchor.workspace as any;
if (workspace.TrsalesLicense) {
  workspace.TrsalesLicense.programId = PROGRAM_ID;
}
if (workspace.trsales_license) {
  workspace.trsales_license.programId = PROGRAM_ID;
}
const program: Program<TrsalesLicense> =
  (workspace.TrsalesLicense as Program<TrsalesLicense>) ??
  (workspace.trsales_license as Program<TrsalesLicense>) ??
  new anchor.Program(idl as anchor.Idl, PROGRAM_ID, provider);

const CONFIG_SEED = Buffer.from("config");
const PROFILE_SEED = Buffer.from("profile");
const REWARD_SEED = Buffer.from("reward");
const ORDER_SEED = Buffer.from("order");

function getConfigPda(): PublicKey {
  return PublicKey.findProgramAddressSync([CONFIG_SEED], PROGRAM_ID)[0];
}

function getProfilePda(user: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([PROFILE_SEED, user.toBuffer()], PROGRAM_ID)[0];
}

function getRewardVaultPda(user: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync([REWARD_SEED, user.toBuffer()], PROGRAM_ID)[0];
}

function getOrderPda(orderId: Buffer): PublicKey {
  return PublicKey.findProgramAddressSync([ORDER_SEED, orderId], PROGRAM_ID)[0];
}

async function ensureAirdrop(pubkey: PublicKey, sol: number = 1): Promise<void> {
  const balance = await provider.connection.getBalance(pubkey);
  const needed = sol * LAMPORTS_PER_SOL;
  if (balance < needed) {
    const sig = await provider.connection.requestAirdrop(pubkey, needed - balance + 1000000);
    await provider.connection.confirmTransaction(sig, "confirmed");
  }
}

async function createAtaIfMissing(mint: PublicKey, owner: PublicKey, payer: Keypair): Promise<PublicKey> {
  const ata = getAssociatedTokenAddressSync(mint, owner, true);
  const info = await provider.connection.getAccountInfo(ata);
  if (!info) {
    const tx = new anchor.web3.Transaction().add(
      createAssociatedTokenAccountInstruction(payer.publicKey, ata, owner, mint)
    );
    await provider.sendAndConfirm(tx, [payer]);
  }
  return ata;
}

function buildReferralRemaining(referrers: Keypair[], collectionMint: PublicKey, usdcMint: PublicKey): anchor.web3.AccountMeta[] {
  const metas: anchor.web3.AccountMeta[] = [];
  for (const kp of referrers) {
    const profile = getProfilePda(kp.publicKey);
    const rewardVault = getRewardVaultPda(kp.publicKey);
    const rewardUsdc = getAssociatedTokenAddressSync(usdcMint, rewardVault, true);
    const nftAccount = getAssociatedTokenAddressSync(collectionMint, kp.publicKey);
    metas.push(
      { pubkey: profile, isWritable: false, isSigner: false },
      { pubkey: rewardVault, isWritable: true, isSigner: false },
      { pubkey: rewardUsdc, isWritable: true, isSigner: false },
      { pubkey: nftAccount, isWritable: false, isSigner: false }
    );
  }
  return metas;
}

describe("trsales_license", () => {
  const admin = provider.wallet;
  const feePayer = Keypair.generate();
  const top = Keypair.generate();
  const mid = Keypair.generate();
  const direct = Keypair.generate();
  const buyer = Keypair.generate();

  let configPda: PublicKey;
  let usdcMint: PublicKey;
  let collectionMint: PublicKey;
  let treasurySol: Keypair;
  let treasuryUsdcAta: PublicKey;
  let solPrice: anchor.BN;
  let usdcPrice: anchor.BN;

  before(async () => {
    configPda = getConfigPda();
    treasurySol = Keypair.generate();

    await ensureAirdrop(admin.publicKey, 2);
    await ensureAirdrop(feePayer.publicKey, 10);
    await ensureAirdrop(top.publicKey, 2);
    await ensureAirdrop(mid.publicKey, 2);
    await ensureAirdrop(direct.publicKey, 2);
    await ensureAirdrop(buyer.publicKey, 2);
    await ensureAirdrop(treasurySol.publicKey, 1);

    usdcMint = await createMint(
      provider.connection,
      feePayer,
      feePayer.publicKey,
      null,
      6
    );

    collectionMint = await createMint(
      provider.connection,
      feePayer,
      configPda,
      null,
      0
    );

    treasuryUsdcAta = getAssociatedTokenAddressSync(usdcMint, admin.publicKey);
    const treasuryInfo = await provider.connection.getAccountInfo(treasuryUsdcAta);
    if (!treasuryInfo) {
      const tx = new anchor.web3.Transaction().add(
        createAssociatedTokenAccountInstruction(
          feePayer.publicKey,
          treasuryUsdcAta,
          admin.publicKey,
          usdcMint
        )
      );
      await provider.sendAndConfirm(tx, [feePayer]);
    }

    const solPriceInit = new anchor.BN(Math.floor(LAMPORTS_PER_SOL / 10));
    const usdcPriceInit = new anchor.BN(50_000_000);

    const configAccount = await provider.connection.getAccountInfo(configPda);
    if (!configAccount) {
      await program.methods
        .initialize({
          admin: admin.publicKey,
          usdcMint,
          collectionMint,
          solPrice: solPriceInit,
          usdcPrice: usdcPriceInit,
        })
        .accounts({
          config: configPda,
          payer: feePayer.publicKey,
          treasurySol: treasurySol.publicKey,
          treasuryUsdc: treasuryUsdcAta,
          collectionMint,
          systemProgram: SystemProgram.programId,
        })
        .signers([feePayer])
        .rpc();
    }

    const cfg = await program.account.config.fetch(configPda);
    solPrice = cfg.solPrice;
    usdcPrice = cfg.usdcPrice;
  });

  async function registerUser(user: Keypair): Promise<void> {
    const profilePda = getProfilePda(user.publicKey);
    const rewardVaultPda = getRewardVaultPda(user.publicKey);
    const rewardUsdcAta = await createAtaIfMissing(usdcMint, rewardVaultPda, feePayer);

    await program.methods
      .registerUser()
      .accounts({
        config: configPda,
        payer: feePayer.publicKey,
        profile: profilePda,
        rewardVault: rewardVaultPda,
        usdcMint,
        rewardUsdc: rewardUsdcAta,
        user: user.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([feePayer, user])
      .rpc();
  }

  it("registers users", async () => {
    await registerUser(top);
    await registerUser(mid);
    await registerUser(direct);
    await registerUser(buyer);

    const profileTop = await program.account.referralProfile.fetch(getProfilePda(top.publicKey));
    const profileMid = await program.account.referralProfile.fetch(getProfilePda(mid.publicKey));
    const profileDirect = await program.account.referralProfile.fetch(getProfilePda(direct.publicKey));
    const profileBuyer = await program.account.referralProfile.fetch(getProfilePda(buyer.publicKey));

    expect(profileTop.user.toBase58()).to.equal(top.publicKey.toBase58());
    expect(profileMid.user.toBase58()).to.equal(mid.publicKey.toBase58());
    expect(profileDirect.user.toBase58()).to.equal(direct.publicKey.toBase58());
    expect(profileBuyer.user.toBase58()).to.equal(buyer.publicKey.toBase58());
    expect(profileBuyer.hasReferrer).to.equal(false);
  });

  it("sets referral chain top -> mid -> direct -> buyer", async () => {
    await program.methods
      .setReferrer(top.publicKey)
      .accounts({
        profile: getProfilePda(mid.publicKey),
        user: mid.publicKey,
        parentProfile: getProfilePda(top.publicKey),
      })
      .signers([mid])
      .rpc();

    await program.methods
      .setReferrer(mid.publicKey)
      .accounts({
        profile: getProfilePda(direct.publicKey),
        user: direct.publicKey,
        parentProfile: getProfilePda(mid.publicKey),
      })
      .signers([direct])
      .rpc();

    await program.methods
      .setReferrer(direct.publicKey)
      .accounts({
        profile: getProfilePda(buyer.publicKey),
        user: buyer.publicKey,
        parentProfile: getProfilePda(direct.publicKey),
      })
      .signers([buyer])
      .rpc();

    const profileBuyer = await program.account.referralProfile.fetch(getProfilePda(buyer.publicKey));
    const profileDirect = await program.account.referralProfile.fetch(getProfilePda(direct.publicKey));
    const profileMid = await program.account.referralProfile.fetch(getProfilePda(mid.publicKey));

    expect(profileBuyer.referrer.toBase58()).to.equal(direct.publicKey.toBase58());
    expect(profileDirect.referrer.toBase58()).to.equal(mid.publicKey.toBase58());
    expect(profileMid.referrer.toBase58()).to.equal(top.publicKey.toBase58());
  });

  it("mints with SOL and distributes referrals", async () => {
    const buyerNftAta = await createAtaIfMissing(collectionMint, buyer.publicKey, feePayer);
    await createAtaIfMissing(collectionMint, direct.publicKey, feePayer);
    await createAtaIfMissing(collectionMint, mid.publicKey, feePayer);
    await createAtaIfMissing(collectionMint, top.publicKey, feePayer);

    const remainingAccounts = buildReferralRemaining([direct, mid, top], collectionMint, usdcMint);

    const orderId = anchor.web3.Keypair.generate().publicKey.toBuffer().slice(0, 16);
    const orderPda = getOrderPda(orderId);

    await program.methods
      .mintWithSol([...orderId])
      .accounts({
        config: configPda,
        treasurySol: treasurySol.publicKey,
        buyer: buyer.publicKey,
        order: orderPda,
        buyerProfile: getProfilePda(buyer.publicKey),
        buyerRewardVault: getRewardVaultPda(buyer.publicKey),
        collectionMint,
        buyerNftAccount: buyerNftAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remainingAccounts)
      .signers([buyer])
      .rpc();

    const order = await program.account.order.fetch(orderPda);
    expect(order.used).to.equal(true);
    expect(order.buyer.toBase58()).to.equal(buyer.publicKey.toBase58());

    const rewardDirect = await program.account.rewardVault.fetch(getRewardVaultPda(direct.publicKey));
    const rewardMid = await program.account.rewardVault.fetch(getRewardVaultPda(mid.publicKey));
    const rewardTop = await program.account.rewardVault.fetch(getRewardVaultPda(top.publicKey));

    const level1 = solPrice.mul(new anchor.BN(5000)).div(new anchor.BN(10000));
    const level2 = solPrice.mul(new anchor.BN(3000)).div(new anchor.BN(10000));
    const level3 = solPrice.mul(new anchor.BN(2000)).div(new anchor.BN(10000));

    expect(rewardDirect.claimableSol.gte(level1)).to.be.true;
    expect(rewardMid.claimableSol.gte(level2)).to.be.true;
    expect(rewardTop.claimableSol.gte(level3)).to.be.true;
  });

  it("mints with USDC and distributes referrals", async () => {
    const buyerNftAta = await createAtaIfMissing(collectionMint, buyer.publicKey, feePayer);
    const buyerUsdcAta = await createAtaIfMissing(usdcMint, buyer.publicKey, feePayer);
    await mintTo(provider.connection, feePayer, usdcMint, buyerUsdcAta, feePayer, usdcPrice.toNumber() + 1_000_000);

    const remainingAccounts = buildReferralRemaining([direct, mid, top], collectionMint, usdcMint);

    const orderId = anchor.web3.Keypair.generate().publicKey.toBuffer().slice(0, 16);
    const orderPda = getOrderPda(orderId);

    await program.methods
      .mintWithUsdc([...orderId])
      .accounts({
        config: configPda,
        treasurySol: treasurySol.publicKey,
        buyer: buyer.publicKey,
        order: orderPda,
        buyerProfile: getProfilePda(buyer.publicKey),
        buyerRewardVault: getRewardVaultPda(buyer.publicKey),
        collectionMint,
        buyerNftAccount: buyerNftAta,
        buyerUsdc: buyerUsdcAta,
        treasuryUsdc: treasuryUsdcAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remainingAccounts)
      .signers([buyer])
      .rpc();

    const rewardDirect = await program.account.rewardVault.fetch(getRewardVaultPda(direct.publicKey));
    const rewardMid = await program.account.rewardVault.fetch(getRewardVaultPda(mid.publicKey));
    const rewardTop = await program.account.rewardVault.fetch(getRewardVaultPda(top.publicKey));

    const level1 = usdcPrice.mul(new anchor.BN(5000)).div(new anchor.BN(10000));
    const level2 = usdcPrice.mul(new anchor.BN(3000)).div(new anchor.BN(10000));
    const level3 = usdcPrice.mul(new anchor.BN(2000)).div(new anchor.BN(10000));

    expect(rewardDirect.claimableUsdc.gte(level1)).to.be.true;
    expect(rewardMid.claimableUsdc.gte(level2)).to.be.true;
    expect(rewardTop.claimableUsdc.gte(level3)).to.be.true;
  });
});
