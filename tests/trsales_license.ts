import * as anchor from "@project-serum/anchor";
import { Program } from "@project-serum/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  createMint,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import {
  Keypair,
  SystemProgram,
  PublicKey,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import { expect } from "chai";
import { TrsalesLicense } from "../target/types/trsales_license";

const provider = anchor.AnchorProvider.env();
anchor.setProvider(provider);

const program = anchor.workspace.trsales_license as Program<TrsalesLicense>;

if (!provider || !provider.wallet || !provider.wallet.publicKey) {
  throw new Error(
    "Provider wallet or wallet.publicKey is undefined. Make sure AnchorProvider.env() is configured with a wallet.",
  );
}
if (!program) {
  throw new Error(
    "Program trsales_license not found in anchor.workspace. Check Anchor.toml and IDL name.",
  );
}

const admin = provider.wallet;
const adminSigner = (admin as anchor.Wallet & { payer: Keypair }).payer;
const connection = provider.connection;

const CONFIG_SEED = Buffer.from("config");
const PROFILE_SEED = Buffer.from("profile");
const REWARD_SEED = Buffer.from("reward");
const ORDER_SEED = Buffer.from("order");

function getConfigPda(): PublicKey {
  return PublicKey.findProgramAddressSync([CONFIG_SEED], program.programId)[0];
}

function getProfilePda(user: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [PROFILE_SEED, user.toBuffer()],
    program.programId,
  )[0];
}

function getRewardVaultPda(user: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [REWARD_SEED, user.toBuffer()],
    program.programId,
  )[0];
}

function getOrderPda(orderIdBytes: Buffer): PublicKey {
  return PublicKey.findProgramAddressSync(
    [ORDER_SEED, orderIdBytes],
    program.programId,
  )[0];
}

const userA = Keypair.generate();
const userB = Keypair.generate();
const userC = Keypair.generate();
const userD = Keypair.generate();
const feePayer = Keypair.generate();

const referralUsers = [userB, userC, userD];

const solPrice = new anchor.BN(Math.floor(LAMPORTS_PER_SOL / 10));
const usdcPrice = new anchor.BN(50_000_000);

const referralBps = [5000, 3000, 2000];
const basisPoints = 10000;

const userData = new Map<
  string,
  { profile: PublicKey; rewardVault: PublicKey; rewardUsdc: PublicKey }
>();

let configPda: PublicKey;
let usdcMint: PublicKey;
let collectionMint: PublicKey;
let treasurySolKp: Keypair;
let treasuryUsdcAta: PublicKey;

describe("trsales_license", () => {
  before(async () => {
    configPda = getConfigPda();

    await airdrop(feePayer.publicKey, 10 * LAMPORTS_PER_SOL);
    await Promise.all(
      [userA, userB, userC, userD].map((kp) =>
        airdrop(kp.publicKey, 2 * LAMPORTS_PER_SOL),
      ),
    );

    treasurySolKp = Keypair.generate();
    await airdrop(treasurySolKp.publicKey, 2 * LAMPORTS_PER_SOL);

    usdcMint = await createMint(
      connection,
      feePayer,
      admin.publicKey,
      null,
      6,
    );

    treasuryUsdcAta = await ensureAta(usdcMint, admin.publicKey, false);

    collectionMint = await createMint(
      connection,
      feePayer,
      configPda,
      configPda,
      0,
    );

    await program.methods
      .initialize({
        admin: admin.publicKey,
        usdcMint,
        collectionMint,
        solPrice,
        usdcPrice,
      })
      .accounts({
        config: configPda,
        payer: admin.publicKey,
        treasurySol: treasurySolKp.publicKey,
        treasuryUsdc: treasuryUsdcAta,
        collectionMint,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
  });

  it("initializes config", async () => {
    const config = await program.account.config.fetch(configPda);
    expect(config.admin.equals(admin.publicKey)).to.be.true;
    expect(config.usdcMint.equals(usdcMint)).to.be.true;
    expect(config.collectionMint.equals(collectionMint)).to.be.true;
    expect(config.treasurySol.equals(treasurySolKp.publicKey)).to.be.true;
    expect(config.treasuryUsdc.equals(treasuryUsdcAta)).to.be.true;
    expect(config.solPrice.eq(solPrice)).to.be.true;
    expect(config.usdcPrice.eq(usdcPrice)).to.be.true;
  });

  it("registers all users", async () => {
    for (const user of [userA, userB, userC, userD]) {
      const profilePda = getProfilePda(user.publicKey);
      const rewardVault = getRewardVaultPda(user.publicKey);
      const rewardUsdc = await ensureAta(usdcMint, rewardVault, true);

      await program.methods
        .registerUser()
        .accounts({
          config: configPda,
          payer: user.publicKey,
          profile: profilePda,
          rewardVault,
          usdcMint,
          rewardUsdc,
          user: user.publicKey,
          systemProgram: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          rent: anchor.web3.SYSVAR_RENT_PUBKEY,
        })
        .signers([user])
        .rpc();

      userData.set(user.publicKey.toBase58(), {
        profile: profilePda,
        rewardVault,
        rewardUsdc,
      });

      const profile = await program.account.referralProfile.fetch(profilePda);
      expect(profile.user.equals(user.publicKey)).to.be.true;
      expect(profile.hasReferrer).to.be.false;

      const rewardVaultAccount = await program.account.rewardVault.fetch(rewardVault);
      expect(rewardVaultAccount.user.equals(user.publicKey)).to.be.true;
      expect(rewardVaultAccount.claimableSol.toNumber()).to.equal(0);
      expect(rewardVaultAccount.claimableUsdc.toNumber()).to.equal(0);
    }
  });

  it("sets referral chain A → B → C → D", async () => {
    await program.methods
      .setReferrer(userB.publicKey)
      .accounts({
        profile: userData.get(userA.publicKey.toBase58())!.profile,
        user: userA.publicKey,
        parentProfile: userData.get(userB.publicKey.toBase58())!.profile,
      })
      .signers([userA])
      .rpc();

    await program.methods
      .setReferrer(userC.publicKey)
      .accounts({
        profile: userData.get(userB.publicKey.toBase58())!.profile,
        user: userB.publicKey,
        parentProfile: userData.get(userC.publicKey.toBase58())!.profile,
      })
      .signers([userB])
      .rpc();

    await program.methods
      .setReferrer(userD.publicKey)
      .accounts({
        profile: userData.get(userC.publicKey.toBase58())!.profile,
        user: userC.publicKey,
        parentProfile: userData.get(userD.publicKey.toBase58())!.profile,
      })
      .signers([userC])
      .rpc();

    const profileA = await program.account.referralProfile.fetch(
      userData.get(userA.publicKey.toBase58())!.profile,
    );
    const profileB = await program.account.referralProfile.fetch(
      userData.get(userB.publicKey.toBase58())!.profile,
    );
    const profileC = await program.account.referralProfile.fetch(
      userData.get(userC.publicKey.toBase58())!.profile,
    );
    const profileD = await program.account.referralProfile.fetch(
      userData.get(userD.publicKey.toBase58())!.profile,
    );

    expect(profileA.referrer.equals(userB.publicKey)).to.be.true;
    expect(profileA.hasReferrer).to.be.true;
    expect(profileB.referrer.equals(userC.publicKey)).to.be.true;
    expect(profileB.hasReferrer).to.be.true;
    expect(profileC.referrer.equals(userD.publicKey)).to.be.true;
    expect(profileC.hasReferrer).to.be.true;
    expect(profileD.hasReferrer).to.be.false;
  });

  it("mints with SOL and distributes referral rewards", async () => {
    const orderIdBuffer = Buffer.alloc(16);
    orderIdBuffer.write("sol-order-0001");
    const orderPda = getOrderPda(orderIdBuffer);
    const buyerProfile = userData.get(userA.publicKey.toBase58())!.profile;
    const buyerRewardVault = userData.get(userA.publicKey.toBase58())!.rewardVault;
    const buyerNftAccount = await ensureAta(collectionMint, userA.publicKey, false);

    const remainingAccounts = await buildReferralAccounts();

    const beforeVaults = await Promise.all(
      referralUsers.map((user) =>
        program.account.rewardVault.fetch(
          userData.get(user.publicKey.toBase58())!.rewardVault,
        ),
      ),
    );
    const treasuryBefore = await connection.getBalance(treasurySolKp.publicKey);

    await program.methods
      .mintWithSol(orderIdBuffer)
      .accounts({
        config: configPda,
        treasurySol: treasurySolKp.publicKey,
        buyer: userA.publicKey,
        order: orderPda,
        buyerProfile,
        buyerRewardVault,
        collectionMint,
        buyerNftAccount,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remainingAccounts)
      .signers([userA])
      .rpc();

    const order = await program.account.order.fetch(orderPda);
    expect(order.used).to.be.true;
    expect(order.buyer.equals(userA.publicKey)).to.be.true;

    const afterVaults = await Promise.all(
      referralUsers.map((user) =>
        program.account.rewardVault.fetch(
          userData.get(user.publicKey.toBase58())!.rewardVault,
        ),
      ),
    );

    const shareB = solPrice.muln(referralBps[0]).divn(basisPoints);
    const shareC = solPrice.muln(referralBps[1]).divn(basisPoints);
    const shareD = solPrice.muln(referralBps[2]).divn(basisPoints);

    expect(
      afterVaults[0].claimableSol.sub(beforeVaults[0].claimableSol).eq(shareB),
    ).to.be.true;
    expect(
      afterVaults[1].claimableSol.sub(beforeVaults[1].claimableSol).eq(shareC),
    ).to.be.true;
    expect(
      afterVaults[2].claimableSol.sub(beforeVaults[2].claimableSol).eq(shareD),
    ).to.be.true;

    for (let i = 0; i < referralUsers.length; i++) {
      const vaultPubkey = userData.get(referralUsers[i].publicKey.toBase58())!
        .rewardVault;
      const lamports = await connection.getBalance(vaultPubkey);
      expect(lamports).to.be.gte(afterVaults[i].claimableSol.toNumber());
    }

    const treasuryAfter = await connection.getBalance(treasurySolKp.publicKey);
    const expectedRemainder = solPrice
      .sub(shareB.add(shareC).add(shareD))
      .toNumber();
    expect(treasuryAfter - treasuryBefore).to.equal(expectedRemainder);
  });

  it("allows referrers to claim SOL", async () => {
    for (const user of referralUsers) {
      const rewardVault = userData.get(user.publicKey.toBase58())!.rewardVault;
      const rewardBefore = await program.account.rewardVault.fetch(rewardVault);
      const userBefore = await connection.getBalance(user.publicKey);

      await program.methods
        .claimSol()
        .accounts({
          rewardVault,
          user: user.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([user])
        .rpc();

      const rewardAfter = await program.account.rewardVault.fetch(rewardVault);
      expect(rewardAfter.claimableSol.toNumber()).to.equal(0);
      const userAfter = await connection.getBalance(user.publicKey);
      const tolerance = 10_000;
      expect(userAfter).to.be.gte(
        userBefore + rewardBefore.claimableSol.toNumber() - tolerance,
      );
    }
  });

  it("mints with USDC and distributes referral rewards", async () => {
    const orderIdBuffer = Buffer.alloc(16);
    orderIdBuffer.write("usdc-order-0001");
    const orderPda = getOrderPda(orderIdBuffer);
    const buyerProfile = userData.get(userA.publicKey.toBase58())!.profile;
    const buyerRewardVault = userData.get(userA.publicKey.toBase58())!.rewardVault;
    const buyerNftAccount = await ensureAta(collectionMint, userA.publicKey, false);
    const buyerUsdc = await ensureAta(usdcMint, userA.publicKey, false);

    await mintTo(
      connection,
      feePayer,
      usdcMint,
      buyerUsdc,
      adminSigner,
      usdcPrice.muln(2).toNumber(),
    );

    const remainingAccounts = await buildReferralAccounts();

    const beforeVaults = await Promise.all(
      referralUsers.map((user) =>
        program.account.rewardVault.fetch(
          userData.get(user.publicKey.toBase58())!.rewardVault,
        ),
      ),
    );
    const beforeRewardBalances = await Promise.all(
      referralUsers.map((user) =>
        getTokenBalance(userData.get(user.publicKey.toBase58())!.rewardUsdc),
      ),
    );
    const treasuryBefore = await getTokenBalance(treasuryUsdcAta);

    await program.methods
      .mintWithUsdc(orderIdBuffer)
      .accounts({
        config: configPda,
        treasurySol: treasurySolKp.publicKey,
        buyer: userA.publicKey,
        order: orderPda,
        buyerProfile,
        buyerRewardVault,
        collectionMint,
        buyerNftAccount,
        buyerUsdc,
        treasuryUsdc: treasuryUsdcAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(remainingAccounts)
      .signers([userA])
      .rpc();

    const afterVaults = await Promise.all(
      referralUsers.map((user) =>
        program.account.rewardVault.fetch(
          userData.get(user.publicKey.toBase58())!.rewardVault,
        ),
      ),
    );
    const afterRewardBalances = await Promise.all(
      referralUsers.map((user) =>
        getTokenBalance(userData.get(user.publicKey.toBase58())!.rewardUsdc),
      ),
    );
    const treasuryAfter = await getTokenBalance(treasuryUsdcAta);

    const shareB = usdcPrice.muln(referralBps[0]).divn(basisPoints);
    const shareC = usdcPrice.muln(referralBps[1]).divn(basisPoints);
    const shareD = usdcPrice.muln(referralBps[2]).divn(basisPoints);

    expect(
      afterVaults[0].claimableUsdc.sub(beforeVaults[0].claimableUsdc).eq(shareB),
    ).to.be.true;
    expect(
      afterVaults[1].claimableUsdc.sub(beforeVaults[1].claimableUsdc).eq(shareC),
    ).to.be.true;
    expect(
      afterVaults[2].claimableUsdc.sub(beforeVaults[2].claimableUsdc).eq(shareD),
    ).to.be.true;

    expect(afterRewardBalances[0].sub(beforeRewardBalances[0]).eq(shareB)).to.be.true;
    expect(afterRewardBalances[1].sub(beforeRewardBalances[1]).eq(shareC)).to.be.true;
    expect(afterRewardBalances[2].sub(beforeRewardBalances[2]).eq(shareD)).to.be.true;

    const totalDistributed = shareB.add(shareC).add(shareD);
    expect(treasuryAfter.sub(treasuryBefore).eq(usdcPrice.sub(totalDistributed))).to.be.true;
  });

  it("allows referrers to claim USDC", async () => {
    for (const user of referralUsers) {
      const rewardVault = userData.get(user.publicKey.toBase58())!.rewardVault;
      const rewardUsdc = userData.get(user.publicKey.toBase58())!.rewardUsdc;
      const userUsdc = await ensureAta(usdcMint, user.publicKey, false);
      const rewardBefore = await program.account.rewardVault.fetch(rewardVault);
      const userBefore = await getTokenBalance(userUsdc);

      await program.methods
        .claimUsdc()
        .accounts({
          rewardVault,
          rewardUsdc,
          userUsdc,
          user: user.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([user])
        .rpc();

      const rewardAfter = await program.account.rewardVault.fetch(rewardVault);
      expect(rewardAfter.claimableUsdc.toNumber()).to.equal(0);
      const userAfter = await getTokenBalance(userUsdc);
      expect(userAfter.sub(userBefore).toNumber()).to.equal(
        rewardBefore.claimableUsdc.toNumber(),
      );
    }
  });
});

async function airdrop(pubkey: PublicKey, amount: number) {
  const sig = await connection.requestAirdrop(pubkey, amount);
  await connection.confirmTransaction(sig, "confirmed");
}

async function ensureAta(
  mint: PublicKey,
  owner: PublicKey,
  allowOwnerOffCurve: boolean,
): Promise<PublicKey> {
  const ata = getAssociatedTokenAddressSync(mint, owner, allowOwnerOffCurve);
  const info = await connection.getAccountInfo(ata);
  if (!info) {
    const ix = createAssociatedTokenAccountInstruction(
      feePayer.publicKey,
      ata,
      owner,
      mint,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const tx = new anchor.web3.Transaction().add(ix);
    await provider.sendAndConfirm(tx, [feePayer]);
  }
  return ata;
}

async function buildReferralAccounts() {
  const accounts: { pubkey: PublicKey; isWritable: boolean; isSigner: boolean }[] = [];
  for (const user of referralUsers) {
    const profile = userData.get(user.publicKey.toBase58())!.profile;
    const rewardVault = userData.get(user.publicKey.toBase58())!.rewardVault;
    const rewardUsdc = userData.get(user.publicKey.toBase58())!.rewardUsdc;
    const nft = await ensureAta(collectionMint, user.publicKey, false);
    accounts.push(
      { pubkey: profile, isWritable: false, isSigner: false },
      { pubkey: rewardVault, isWritable: true, isSigner: false },
      { pubkey: rewardUsdc, isWritable: false, isSigner: false },
      { pubkey: nft, isWritable: false, isSigner: false },
    );
  }
  return accounts;
}

async function getTokenBalance(ata: PublicKey) {
  const account = await getAccount(connection, ata);
  return new anchor.BN(account.amount.toString());
}
