<!--
Copyright 2026 coccinella-labs

Licensed under the Apache License, Version 2.0 (the "License");
you may not use this file except in compliance with the License.
You may obtain a copy of the License at

    http://www.apache.org/licenses/LICENSE-2.0

Unless required by applicable law or agreed to in writing, software
distributed under the License is distributed on an "AS IS" BASIS,
WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
See the License for the specific language governing permissions and
limitations under the License.
-->

# Privacy Policy

[![Privacy: Local First](https://img.shields.io/badge/privacy-local%20first-blue)](https://github.com/coccinella-labs/harper)
[![Data: Encrypted Storage](https://img.shields.io/badge/data-encrypted%20storage-green)](https://github.com/coccinella-labs/harper)

Harper is committed to protecting your privacy. This policy explains how we handle your data and what you can expect from our privacy practices.

## Table of Contents

- [Our Commitment](#our-commitment)
- [Data Collection](#data-collection)
- [Data Storage](#data-storage)
- [Data Usage](#data-usage)
- [Data Sharing](#data-sharing)
- [Your Rights](#your-rights)
- [Security Measures](#security-measures)
- [Third-Party Services](#third-party-services)
- [Data Retention](#data-retention)
- [International Data Transfers](#international-data-transfers)
- [Changes to This Policy](#changes-to-this-policy)
- [Contact Us](#contact-us)

## Our Commitment

Harper follows a **privacy-first** approach:

- **Local Storage**: All conversation data stays on your device
- **No Tracking**: We don't track your usage or collect analytics
- **User Control**: You have complete control over your data
- **Transparency**: We're open source, so you can verify our claims
- **Minimal Data**: We collect only what's necessary for functionality

## Data Collection

### What We Collect

Harper collects only the minimum data required for functionality:

#### Conversation Data
- **Chat messages**: Your conversations with AI assistants
- **Session metadata**: Timestamps, session IDs, and basic statistics
- **User preferences**: Your chosen AI provider and model settings

#### Technical Data
- **Configuration**: Your API provider preferences and settings
- **Error logs**: Technical errors for debugging (stored locally)
- **Performance metrics**: Response times and usage statistics (local only)

### What We Don't Collect

- **Personal information**: Names, emails, or identifying information
- **Usage analytics**: How you use Harper or what you discuss
- **Location data**: Your geographic location or IP address
- **Device information**: Hardware details or system specifications
- **Behavioral data**: Patterns of usage or preferences

## Data Storage

### Local Storage Only

- **Database**: SQLite database stored locally on your device
- **Location**: Default location is `./chat_sessions.db` (configurable)
- **Encryption**: All sensitive data is encrypted using AES-GCM-256
- **Access**: Only you can access your data; we have no remote access

### Configuration Files

- **API Keys**: Stored securely in environment variables or config files
- **Settings**: Provider preferences and application settings
- **Location**: Local configuration files in your project directory

## Data Usage

### How We Use Your Data

Your data is used exclusively for:

- **AI Conversations**: Processing your messages with chosen AI providers
- **Session Management**: Organizing and retrieving your chat history
- **Application Functionality**: Providing the core features of Harper
- **Error Resolution**: Debugging technical issues (local only)

### Data Processing

- **Local Processing**: All data processing happens on your device
- **AI Provider APIs**: Messages are sent to your chosen AI provider for processing
- **No Aggregation**: We don't combine or analyze data across users
- **No Profiling**: We don't create user profiles or behavioral patterns

## Data Sharing

### What We Share

We share your data only in these limited circumstances:

#### AI Provider APIs
- **Purpose**: To provide AI responses to your messages
- **Providers**: OpenAI, Sambanova, Google Gemini, or Cerebras (your choice)
- **Data Sent**: Only the conversation messages you send
- **Retention**: Subject to each provider's privacy policy

#### Public Sharing
- **Never**: We never share your conversation data publicly
- **Opt-in Only**: You control all data export and sharing
- **Anonymized**: Any shared technical data is fully anonymized

### What We Don't Share

- **Conversation History**: Never shared with third parties
- **API Keys**: Never transmitted except to authorized AI providers
- **Personal Data**: We don't collect personal data to share
- **Analytics**: No usage data is shared with anyone

## Your Rights

### Data Control Rights

You have complete control over your data:

#### Access Your Data
```bash
# View previous sessions
harper  # Open History from the Home screen

# View specific session
harper  # Open History, select a session, then preview or resume it
```

#### Export Your Data
```bash
# Export session history
harper  # Open Export from the Home screen
```

#### Delete Your Data
- **Individual Sessions**: Delete specific conversations
- **All Data**: Remove the SQLite database file
- **Configuration**: Delete or modify config files

### Data Portability

- **Export Formats**: JSON, CSV, and plain text
- **Migration**: Easy to move data between Harper installations
- **Backup**: Regular backups recommended for important conversations

## Security Measures

### Encryption
- **AES-GCM-256**: Industry-standard encryption for stored data
- **Key Management**: Secure key derivation and storage
- **Memory Safety**: Rust prevents common memory vulnerabilities

### Access Controls
- **Local Access**: Data accessible only from your device
- **File Permissions**: Database files have appropriate permissions
- **API Security**: Secure communication with AI providers

### Security Practices
- **Regular Audits**: Code security reviews and dependency checks
- **Vulnerability Management**: Prompt fixes for security issues
- **Secure Defaults**: Conservative security settings by default

## Third-Party Services

### AI Providers

Harper integrates with third-party AI services:

| Provider | Privacy Policy | Data Usage |
|----------|----------------|------------|
| **OpenAI** | [OpenAI Privacy](https://openai.com/privacy/) | Messages processed for AI responses |
| **Sambanova** | [Sambanova Privacy](https://sambanova.ai/privacy/) | Messages processed for AI responses |
| **Google Gemini** | [Google Privacy](https://policies.google.com/privacy) | Messages processed for AI responses |
| **Cerebras** | [Cerebras Privacy](https://cerebras.ai/privacy) | Messages processed for AI responses |

### Important Notes

- **Your Choice**: You select which provider to use
- **Direct Communication**: Messages go directly from Harper to the provider
- **Provider Policies**: Each provider has its own privacy policy
- **No Additional Sharing**: We don't share data beyond what's sent to providers

## Data Retention

### Retention Policy

- **Conversation Data**: Retained until you delete it
- **Configuration**: Retained until you modify settings
- **Logs**: Temporary logs deleted on application restart

### Data Deletion

You can delete your data at any time:

```bash
# Remove database
rm chat_sessions.db

# Remove configuration
rm -rf config/local.toml
rm .env
```

## International Data Transfers

### Data Location

- **Primary Location**: Data stays on your local device
- **AI Processing**: May occur in provider's data centers
- **No Cross-Border Transfers**: We don't transfer data between countries

### Compliance

- **GDPR**: We comply with GDPR requirements for data protection
- **Local Laws**: Subject to laws in your jurisdiction
- **Provider Compliance**: AI providers handle their own compliance

## Changes to This Policy

### Policy Updates

- **Notification**: Significant changes will be announced via:
  - GitHub releases
  - Changelog updates
  - Documentation updates

- **Version History**: All changes tracked in git history
- **Effective Date**: Changes take effect immediately upon release

### Your Options

If you disagree with policy changes:
- **Continue Using**: Accept the new terms
- **Stop Using**: Delete your data and stop using Harper
- **Contact Us**: Discuss concerns via GitHub issues

## Contact Us

### Privacy Questions

- **GitHub Issues**: [Open a privacy-related issue](https://github.com/coccinella-labs/harper/issues)
- **Discussions**: [Privacy discussions](https://github.com/coccinella-labs/harper/discussions)
- **Email**: coccinella.labs@icloud.com (for sensitive concerns)

### Data Requests

To exercise your data rights:
1. Open a GitHub issue with "Privacy Request" in the title
2. Specify what action you need (access, export, deletion)
3. Provide verification details if required

---

**Last Updated**: November 2025
**Version**: 1.0

*This privacy policy applies to Harper and is separate from any AI provider privacy policies.*
