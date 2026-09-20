# Keychain

Harper can securely store your API keys in your operating system's keychain instead of plain text config files.

## Why Use Keychain?

* **Security**: API keys are encrypted and stored in your OS's secure storage
* **Convenience**: No need to edit config files to change keys
* **Portability**: Keys persist across config file changes

## How It Works

Harper uses the OS keychain to store API keys:
* **macOS**: Keychain Access
* **Linux**: Secret Service API (e.g., GNOME Keyring)
* **Windows**: Credential Manager

## Commands

### Login

Store an API key in your keychain:

```bash
harper auth login --provider openai
harper auth login --provider sambanova
harper auth login --provider gemini
```

You'll be prompted to enter your API key.

### Logout

Remove an API key from your keychain:

```bash
harper auth logout --provider openai
```

## Setup

No additional setup needed. The keyring library handles OS integration automatically.

## Troubleshooting

### Keychain Access Denied

On Linux, you may need to install a secret service backend:
```bash
# Ubuntu/Debian
sudo apt-get install libsecret-1-0 libsecret-tools

# Fedora
sudo dnf install libsecret
```

### Multiple Accounts

Each provider can only have one key stored. Use logout then login to replace.

## Security

Keys stored in the keychain are:
* Encrypted by your OS
* Not stored in config files
* Protected by your user account
