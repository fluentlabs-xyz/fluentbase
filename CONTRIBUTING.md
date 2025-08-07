# Contributing Guide

Thank you for considering contributing to this project! This guide will help you understand our development workflow and
standards.

## Table of Contents

- [Contributing Guide](#contributing-guide)
    - [Table of Contents](#table-of-contents)
    - [Code of Conduct](#code-of-conduct)
    - [Getting Started](#getting-started)
        - [Prerequisites](#prerequisites)
        - [Development Setup](#development-setup)
    - [Development Workflow](#development-workflow)
        - [1. Creating Issues](#1-creating-issues)
        - [2. Branch Creation](#2-branch-creation)
        - [3. Development Process](#3-development-process)
        - [4. Pull Request Process](#4-pull-request-process)
    - [Commit Guidelines](#commit-guidelines)
    - [Release Process](#release-process)
        - [Version Numbering](#version-numbering)
        - [Creating a Release](#creating-a-release)
        - [Managing Published Versions](#managing-published-versions)
            - [Yanking a Release](#yanking-a-release)
    - [Getting Help](#getting-help)

## Code of Conduct

We follow the [Contributor Covenant](https://www.contributor-covenant.org/). We expect all contributors to be respectful
and inclusive in all interactions. Key points:

- Use welcoming and inclusive language
- Respect different viewpoints and experiences
- Accept constructive criticism gracefully
- Focus on what's best for the community

## Getting Started

### Prerequisites

- Rust toolchain
- Git
- Make

### Development Setup

1. **Fork and Clone**

    ```bash
    git clone https://github.com/fluentlabs-xyz/fluentbase.git
    cd fluentbase
    ```

2. **Install Tools**

    ```bash
    make
    ```

   Make sure you have `wasm32-unknown-unknown` target installed using rustup.

    ```bash
    rustup target add wasm32-unknown-unknown
    ```

## Development Workflow

### 1. Creating Issues

Before starting work, ensure there's an issue that describes the change you want to make:

- For bugs: Describe the problem and include reproduction steps
- For features: Explain the proposed functionality and its value
- For improvements: Outline what you want to improve and why

### 2. Branch Creation

Create a feature branch from `main`:

```bash
git checkout -b <type>/<description>

# Examples
git checkout -b feat/user-auth
git checkout -b fix/memory-leak
git checkout -b docs/setup-guide
```

### 3. Development Process

1. **Make Changes**
    - Write code
    - Add tests
    - Update documentation

2. **Local Verification**

   ```bash
   make check # Run all checks
   make test # Run all tests
   ```

3. **Staying Updated**

   ```bash
   git remote add fluentbase https://github.com/fluentlabs-xyz/fluentbase.git
   git pull fluentbase main
   ```

### 4. Pull Request Process

1. **Prepare Changes**
    - Ensure all checks pass
    - Update documentation if needed
    - Add tests for new functionality

2. **Create Pull Request**
    - Use a clear title following commit conventions
    - Fill out the PR template
    - Link related issues
    - Request reviews

3. **Review Process**
    - Address review feedback
    - Keep changes focused
    - Maintain a respectful dialogue

## Commit Guidelines

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>[optional scope]: <description>

# Types
feat:     New features
fix:      Bug fixes
docs:     Documentation changes
style:    Code style/formatting
refactor: Code refactoring
test:     Test updates
chore:    Maintenance tasks

# Examples
feat(auth): add user authentication
fix(mem): correct memory leak
docs: update setup instructions
```

## Release Process

### Version Numbering

We follow [Semantic Versioning](https://semver.org/):

- MAJOR (x.0.0): Breaking changes
- MINOR (0.x.0): New features (backwards compatible)
- PATCH (0.0.x): Bug fixes (backwards compatible)

### Creating a Release

Important notes:

- Releases should only be created from the `main` branch
- Pushing a version tag (e.g., `v1.2.3`) to the `main` branch will automatically trigger the publish workflow
- Ensure all changes are merged to `main` before creating a release

1. **Switch to main and update**

   ```bash
   git checkout main
   git pull origin main
   ```

2. **Verify Changes**

   ```bash
   make check
   ```

3. **Create Release**

   ```bash
   # Using semantic versioning
   make release VERSION=major  # Breaking changes
   make release VERSION=minor  # New features
   make release VERSION=patch  # Bug fixes

   # Or specific version
   make release VERSION=1.2.3
   ```

4. **Review and Push**

   ```bash
   # Review release commit and changelog
   git show                  # Review version bump and commit message
   git diff CHANGELOG.md     # Review changelog updates

   # Make sure changelog entries are:
   # - Properly categorized (breaking changes, features, fixes)
   # - Have clear and descriptive messages
   # - Include relevant issue/PR references

   # Push commits first
   git push origin main

   # After CI verification passes, push tag to trigger publication
   git push origin v1.2.3
   ```

   > **Important**: Pushing the version tag to `main` will automatically trigger
   > the publish workflow in GitHub Actions. Make sure all tests pass before
   > pushing the tag.

5. **Verify Publication**
    - Check GitHub Actions for successful workflow completion
    - Verify the new version appears on crates.io

6. **Reverting Release Preparation** (if needed before publishing)

   ```bash
   make release-undo
   ```

   > **Note**: `release-undo` only reverts local release preparation (commit and tag)
   > and should be used BEFORE pushing changes. It cannot and will not:
   > - Remove a published version from crates.io
   > - Delete remote tags
   > - Revert pushed commits
   >
   > For managing already published versions, see the "Managing Published Versions"
   > section below.

### Managing Published Versions

#### Yanking a Release

If you need to mark a published version as unsuitable for new users (e.g., due to critical bugs):

```bash
# Yank version
cargo yank --version 1.2.3

# Undo yank if needed
cargo yank --version 1.2.3 --undo
```

Important notes about yanking:

- Yanking does NOT delete the version from crates.io
- Existing projects can still use yanked versions
- New projects cannot add yanked versions as dependencies
- Use yanking for versions with:
    - Critical bugs
    - Security vulnerabilities
    - Compatibility issues
    - Accidental publications

When yanking a version:

1. First yank the problematic version
2. Create a new patch release with fixes
3. Update security advisory if needed
4. Notify users through GitHub Issues
5. Update release notes to indicate the version is yanked and explain why

## Getting Help

- **Issues**: Best for bug reports and feature requests
- **Pull Requests**: For code review and feedback

Remember:

- Be clear and specific in questions
- Provide context and examples
- Be patient with responses
- Help others when you can

---

We value all contributions, whether they're:

- Bug reports
- Feature requests
- Code contributions
- Documentation improvements
- Review comments

Every contributor was once a beginner—don't hesitate to ask for help!
