CREATE TABLE crgs_members (
    community_id TEXT NOT NULL,
    member_id BLOB NOT NULL CHECK (typeof(member_id) = 'blob' AND length(member_id) > 0),
    display_handle TEXT,
    skeleton TEXT,
    role TEXT NOT NULL CHECK (role IN ('root', 'admin', 'member')),
    lease_end TEXT NOT NULL CHECK (
        lease_end GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'
        AND substr(lease_end, 1, 4) BETWEEN '0001' AND '9999'
        AND substr(lease_end, 6, 2) BETWEEN '01' AND '12'
    ),
    release_on TEXT CHECK (
        release_on IS NULL OR (
            release_on GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'
            AND substr(release_on, 1, 4) BETWEEN '0001' AND '9999'
            AND substr(release_on, 6, 2) BETWEEN '01' AND '12'
            AND release_on > lease_end
        )
    ),
    PRIMARY KEY (community_id, member_id),
    CHECK (
        (display_handle IS NULL AND skeleton IS NULL AND release_on IS NULL)
        OR (display_handle IS NOT NULL AND length(display_handle) > 0
            AND skeleton IS NOT NULL AND length(skeleton) > 0 AND release_on IS NOT NULL)
    )
) WITHOUT ROWID;
CREATE INDEX crgs_member_release ON crgs_members (community_id, release_on, member_id);

CREATE TABLE crgs_claims (
    community_id TEXT NOT NULL,
    skeleton TEXT NOT NULL CHECK (length(skeleton) > 0),
    kind TEXT NOT NULL CHECK (kind IN ('reserved', 'active', 'blocked')),
    owner BLOB CHECK (owner IS NULL OR (typeof(owner) = 'blob' AND length(owner) > 0)),
    display_handle TEXT,
    expires_at INTEGER,
    blocked_until TEXT CHECK (
        blocked_until IS NULL OR (
            blocked_until GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]'
            AND substr(blocked_until, 1, 4) BETWEEN '0001' AND '9999'
            AND substr(blocked_until, 6, 2) BETWEEN '01' AND '12'
        )
    ),
    PRIMARY KEY (community_id, skeleton),
    CHECK (
        (kind = 'reserved' AND owner IS NOT NULL AND display_handle IS NOT NULL
            AND length(display_handle) > 0 AND expires_at IS NOT NULL AND blocked_until IS NULL)
        OR (kind = 'active' AND owner IS NOT NULL AND display_handle IS NULL
            AND expires_at IS NULL AND blocked_until IS NULL)
        OR (kind = 'blocked' AND owner IS NULL AND display_handle IS NULL
            AND expires_at IS NULL AND blocked_until IS NOT NULL)
    )
) WITHOUT ROWID;
CREATE UNIQUE INDEX crgs_reservation_owner ON crgs_claims (community_id, owner) WHERE kind = 'reserved';
CREATE UNIQUE INDEX crgs_active_owner ON crgs_claims (community_id, owner) WHERE kind = 'active';
CREATE INDEX crgs_reservation_expiry ON crgs_claims (community_id, kind, expires_at, skeleton);
CREATE INDEX crgs_block_expiry ON crgs_claims (community_id, kind, blocked_until, skeleton);
