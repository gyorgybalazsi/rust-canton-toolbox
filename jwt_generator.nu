#!/usr/bin/env nu

# Generate JWT header and payload
# Usage: nu jwt_generator.nu <user_id> [output_file]

def main [user_id: string, output_file: string = "jwt.json"] {
    # JWT header
    let header = {
        alg: "none",
        typ: "JWT"
    }

    # Calculate expiration time (24 hours from now)
    let exp = (date now | date to-timezone utc | into int) / 1000000000 + (24 * 60 * 60)

    # JWT payload
    let payload = {
        aud: "someParticipantId",
        sub: $user_id,
        iss: "someIdpId",
        scope: "daml_ledger_api",
        exp: $exp
    }

    # Encode header and payload with URL-safe base64 (no padding)
    let header_json = ($header | to json --raw)
    let payload_json = ($payload | to json --raw)
    
    let header_enc = ($header_json | encode base64 --url | str replace --all '=' '')
    let payload_enc = ($payload_json | encode base64 --url | str replace --all '=' '')
    
    # Create JWT token (header.payload with no signature for alg "none")
    let jwt_token = $"($header_enc).($payload_enc)."
    
    # Save to file
    $jwt_token | save -f $output_file
    
    # Print confirmation
    print $"JWT saved to ($output_file)"
    print "Header:"
    print ($header | to json)
    print ""
    print "Payload:"
    print ($payload | to json)
    print ""
    print "Encoded JWT Token:"
    print $jwt_token
    
    $jwt_token
}
