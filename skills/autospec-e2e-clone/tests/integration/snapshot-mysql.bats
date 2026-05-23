#!/usr/bin/env bats
# Integration tests for snapshot/mysql.sh
# Requires docker to spin up an ephemeral MySQL container.
# Skip gracefully if docker is unavailable.

SCRIPT_DIR="$(cd "$(dirname "$BATS_TEST_FILENAME")/../.." && pwd)/scripts/snapshot"

MYSQL_CONTAINER_NAME="autospec-test-mysql-$$"
MYSQL_PORT_HOST="13306"
export MYSQL_HOST="127.0.0.1"
export MYSQL_PORT="$MYSQL_PORT_HOST"
export MYSQL_USER="testuser"
export MYSQL_PASSWORD="testpass"
export MYSQL_ROOT_PASSWORD="rootpass"
export MYSQL_DATABASE="testdb"

setup_file() {
    if ! command -v docker >/dev/null 2>&1; then
        return 0
    fi
    docker run -d \
        --name "$MYSQL_CONTAINER_NAME" \
        -e MYSQL_ROOT_PASSWORD="$MYSQL_ROOT_PASSWORD" \
        -e MYSQL_USER="$MYSQL_USER" \
        -e MYSQL_PASSWORD="$MYSQL_PASSWORD" \
        -e MYSQL_DATABASE="$MYSQL_DATABASE" \
        -p "${MYSQL_PORT_HOST}:3306" \
        mysql:8-debian \
        >/dev/null 2>&1 || true

    # Wait for MySQL to be ready (up to 60s)
    local i=0
    while [ $i -lt 60 ]; do
        if docker exec "$MYSQL_CONTAINER_NAME" mysqladmin ping -u root -p"$MYSQL_ROOT_PASSWORD" --silent 2>/dev/null; then
            break
        fi
        sleep 2
        i=$((i + 2))
    done

    # Seed test data
    docker exec "$MYSQL_CONTAINER_NAME" mysql -u root -p"$MYSQL_ROOT_PASSWORD" "$MYSQL_DATABASE" -e "
        CREATE TABLE IF NOT EXISTS users (id INT AUTO_INCREMENT PRIMARY KEY, name VARCHAR(100), email VARCHAR(100));
        INSERT INTO users (name, email) VALUES ('Alice', 'alice@example.com'), ('Bob', 'bob@example.com');
        CREATE TABLE IF NOT EXISTS orders (id INT AUTO_INCREMENT PRIMARY KEY, user_id INT, total DECIMAL(10,2));
        INSERT INTO orders (user_id, total) VALUES (1, 49.99), (2, 19.99);
    " >/dev/null 2>&1 || true
}

teardown_file() {
    if command -v docker >/dev/null 2>&1; then
        docker rm -f "$MYSQL_CONTAINER_NAME" >/dev/null 2>&1 || true
    fi
}

setup() {
    TMP_DIR="$(mktemp -d)"
}

teardown() {
    rm -rf "$TMP_DIR"
}

@test "mysql.sh: schema.sql created for mysql source" {
    require_docker_and_mysql
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/mysql.sh" "users" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/schema.sql" ]
}

@test "mysql.sh: full table dump creates <table>.sql" {
    require_docker_and_mysql
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/mysql.sh" "users" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/users.sql" ]
}

@test "mysql.sh: multiple full tables dumped" {
    require_docker_and_mysql
    OUT_DIR="$TMP_DIR/out"
    run bash "$SCRIPT_DIR/mysql.sh" "users,orders" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -f "$OUT_DIR/users.sql" ]
    [ -f "$OUT_DIR/orders.sql" ]
}

@test "mysql.sh: fails with exit 2 if MYSQL_DATABASE not set" {
    OUT_DIR="$TMP_DIR/out"
    run env -u MYSQL_DATABASE -u DSN bash "$SCRIPT_DIR/mysql.sh" "users" "{}" "$OUT_DIR"
    [ "$status" -eq 2 ]
}

@test "mysql.sh: fails with exit 1 if OUT_DIR not provided" {
    run bash "$SCRIPT_DIR/mysql.sh"
    [ "$status" -eq 1 ]
}

@test "mysql.sh: out_dir is created if it does not exist" {
    require_docker_and_mysql
    OUT_DIR="$TMP_DIR/deep/nested/out"
    run bash "$SCRIPT_DIR/mysql.sh" "" "{}" "$OUT_DIR"
    [ "$status" -eq 0 ]
    [ -d "$OUT_DIR" ]
}

@test "dispatch.sh: routes mysql source correctly" {
    require_docker_and_mysql
    OUT_BASE="$TMP_DIR/clone-snapshots"
    export TEST_MYSQL_DSN="mysql://${MYSQL_USER}:${MYSQL_PASSWORD}@${MYSQL_HOST}:${MYSQL_PORT}/${MYSQL_DATABASE}"
    CONTRACT_JSON=$(cat <<JSON
{
  "sources": [
    {
      "kind": "mysql",
      "dsn_env": "TEST_MYSQL_DSN",
      "tables_full": ["users"]
    }
  ],
  "expose": { "kind": "docker_compose" }
}
JSON
)
    DISPATCH="$SCRIPT_DIR/dispatch.sh"
    run bash -c "printf '%s' \"\$CONTRACT_JSON\" | bash '$DISPATCH' '$OUT_BASE'"
    [ "$status" -eq 0 ]
    SCHEMA_COUNT=$(find "$OUT_BASE" -name "schema.sql" | wc -l | tr -d ' ')
    [ "$SCHEMA_COUNT" -ge 1 ]
}

# Helpers
require_docker_and_mysql() {
    if ! command -v docker >/dev/null 2>&1; then
        skip "docker not available"
    fi
    if ! command -v mysqldump >/dev/null 2>&1; then
        skip "mysqldump not available"
    fi
    if ! docker ps --filter "name=$MYSQL_CONTAINER_NAME" --format "{{.Names}}" | grep -q "$MYSQL_CONTAINER_NAME"; then
        skip "mysql container not running"
    fi
}
