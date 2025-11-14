// Anchor deployment script
import * as anchor from "@coral-xyz/anchor";

module.exports = async function (provider) {
  anchor.setProvider(provider);
  console.log("Deploying trsales_license program...");
};
