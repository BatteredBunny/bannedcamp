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
  pname = "bannedcamp";
  version = "1.1.0";

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
    installShellCompletion --cmd bannedcamp \
      --bash <($out/bin/bannedcamp completions bash) \
      --fish <($out/bin/bannedcamp completions fish) \
      --zsh <($out/bin/bannedcamp completions zsh)
  '';

  meta = with lib; {
    description = "Bandcamp library downloader with TUI";
    homepage = "https://github.com/BatteredBunny/bannedcamp";
    license = licenses.mit;
    mainProgram = "bannedcamp";
  };
}
