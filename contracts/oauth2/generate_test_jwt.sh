#!/bin/bash
# Generate Test JWT Tokens for OAuth2 Contract Testing
#
# Usage:
#   ./generate_test_jwt.sh google test_user_123 client-id test-nonce
#
# This creates test JWTs that can be used for development and testing

PROVIDER=${1:-google}
SUB=${2:-test_user_123}
AUD=${3:-test-client-id}
NONCE=${4:-test-nonce}

# Determine issuer based on provider
case $PROVIDER in
    google)
        ISS="https://accounts.google.com"
        EMAIL="test@gmail.com"
        ;;
    apple)
        ISS="https://appleid.apple.com"
        EMAIL="test@icloud.com"
        ;;
    microsoft)
        ISS="https://login.microsoftonline.com"
        EMAIL="test@outlook.com"
        ;;
    *)
        echo "Unknown provider: $PROVIDER"
        echo "Supported: google, apple, microsoft"
        exit 1
        ;;
esac

# Create header
HEADER=$(echo -n '{"alg":"RS256","typ":"JWT","kid":"test-key-id"}' | base64 | tr -d '=' | tr '+/' '-_')

# Create payload
PAYLOAD=$(cat <<EOF | base64 | tr -d '=' | tr -d '\n' | tr '+/' '-_'
{"iss":"$ISS","sub":"$SUB","aud":"$AUD","exp":9999999999,"iat":1728864000,"nonce":"$NONCE","email":"$EMAIL","email_verified":true}
EOF
)

# Fake signature for testing (won't verify but tests parsing)
SIGNATURE=$(echo -n "fake_test_signature" | base64 | tr -d '=' | tr '+/' '-_')

# Construct JWT
JWT="$HEADER.$PAYLOAD.$SIGNATURE"

echo "Generated Test JWT:"
echo "$JWT"
echo ""
echo "To use in tests:"
echo "export TEST_JWT='$JWT'"

