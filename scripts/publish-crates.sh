#!/bin/bash
set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

# Ensure we are in the project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$ROOT_DIR"

echo -e "${YELLOW}Starting crate publication process...${NC}"

# Check if crates.io is reachable
if ! curl -s --head "https://crates.io" > /dev/null; then
    echo -e "${RED}Error: crates.io is not reachable. Please check your internet connection.${NC}"
    exit 1
fi

# Define crates in order: core, then backends, then meta-crate
# Core must be published first as others depend on it
CRATES=(
    "distributed-lock-core:crates/distributed-lock-core"
    "distributed-lock-file:crates/distributed-lock-file"
    "distributed-lock-mongo:crates/distributed-lock-mongo"
    "distributed-lock-mysql:crates/distributed-lock-mysql"
    "distributed-lock-postgres:crates/distributed-lock-postgres"
    "distributed-lock-redis:crates/distributed-lock-redis"
    "distributed-lock:crates/distributed-lock"
)

# Function to check if a version of a crate is already published
is_published() {
    local crate_name=$1
    local version=$2
    
    # Check if crate exists and get versions
    # Using -i to see headers for potential rate limit info
    local response_file=$(mktemp)
    local http_code=$(curl -s -w "%{http_code}" -o "$response_file" "https://crates.io/api/v1/crates/$crate_name/versions")
    
    if [ "$http_code" -eq 429 ]; then
        echo -e "${YELLOW}Warning: Hit crates.io rate limit (429). Waiting 60s...${NC}" >&2
        sleep 60
        return 1
    fi

    if [ "$http_code" -ne 200 ]; then
        rm -f "$response_file"
        return 1 # Not found or error
    fi
    
    local is_present=$(jq -r '.versions[].num' "$response_file" | grep -qx "$version"; echo $?)
    rm -f "$response_file"
    
    if [ "$is_present" -eq 0 ]; then
        return 0 # Already published
    else
        return 1 # Not published
    fi
}

for CRATE_INFO in "${CRATES[@]}"; do
    IFS=":" read -r NAME CRATE_DIR <<< "$CRATE_INFO"
    
    echo -e "\n${YELLOW}--- Checking $NAME ---${NC}"
    
    # Get local version using cargo read-manifest
    LOCAL_VERSION=$(cargo read-manifest --manifest-path "$CRATE_DIR/Cargo.toml" | jq -r .version)
    echo "Local version: $LOCAL_VERSION"
    
    if is_published "$NAME" "$LOCAL_VERSION"; then
        echo -e "${GREEN}Version $LOCAL_VERSION of $NAME is already published on crates.io.${NC}"
    else
        echo -e "${RED}Version $LOCAL_VERSION of $NAME is NOT published. Publishing...${NC}"
        
        # Perform the publication
        (cd "$CRATE_DIR" && cargo publish)
        
        if [ $? -eq 0 ]; then
            echo -e "${GREEN}Successfully published $NAME $LOCAL_VERSION${NC}"
            
            # Wait for crates.io to index the new version
            echo "Waiting for $NAME $LOCAL_VERSION to become visible on crates.io..."
            # Initial wait before starting to poll
            echo "Wait 60s for initial indexing..."
            sleep 60

            ATTEMPTS=0
            MAX_ATTEMPTS=30
            while ! is_published "$NAME" "$LOCAL_VERSION"; do
                ATTEMPTS=$((ATTEMPTS + 1))
                if [ $ATTEMPTS -ge $MAX_ATTEMPTS ]; then
                    echo -e "${RED}Timed out waiting for $NAME $LOCAL_VERSION to be indexed.${NC}"
                    exit 1
                fi
                echo "Attempt $ATTEMPTS/$MAX_ATTEMPTS: Still waiting for indexing... (sleeping 30s)"
                sleep 30
            done
            echo -e "${GREEN}$NAME $LOCAL_VERSION is now visible!${NC}"
        else
            echo -e "${RED}Failed to publish $NAME $LOCAL_VERSION${NC}"
            exit 1
        fi
    fi
done

echo -e "\n${GREEN}Publication process completed successfully!${NC}"
