{
  pkgs,
  lib ? pkgs.lib,
  rustPlatform,
  pkg-config,
  openssl,
  installShellFiles,
  stdenv,
}:

rustPlatform.buildRustPackage {
  pname = "bannedcam";
  version = "1.0.0";

  src = ./.;

  cargoLock.lockFile = ./Cargo.lock;

  nativeBuildInputs = [
    pkg-config
    installShellFiles
  ];

  buildInputs = [
    openssl
  ];

  postInstall = lib.optionalString (stdenv.buildPlatform.canExecute stdenv.hostPlatform) ''
    installShellCompletion --cmd bannedcam \
      --bash <($out/bin/bannedcam completions bash) \
      --fish <($out/bin/bannedcam completions fish) \
      --zsh <($out/bin/bannedcam completions zsh)
  '';

  meta = with lib; {
    description = "Bandcamp library downloader with TUI";
    homepage = "https://github.com/BatteredBunny/bannedcam";
    license = licenses.mit;
    mainProgram = "bannedcam";
  };
}
