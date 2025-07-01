use clap::Parser;

#[derive(Parser)]
#[command(name = "warp-keygen")]
#[command(about = "Generate keys serialized for use with *warp*")]
struct Args {
    // RegEx to search for in the public key
    //
    // Note: The pattern may be found anywhere in the string; use ^ or $ to anchor to the beginning/end respectively
    //
    // Note: Not all letters are present in the serialisation alphabet (i, l, o, u) to avoid ambiguous characters
    //
    // Note: The public key has a very high likelihood of beginning with '0'
    #[arg()]
    regex: Option<String>,
}

fn main() -> Result<(), anyhow::Error> {
    let args = Args::parse();

    let re = args.regex.unwrap_or_else(|| ".*".to_owned());
    let re = regex::RegexBuilder::new(&re).case_insensitive(true).build()?;

    println!("Searching for {}", re.as_str());

    loop {
        let private_key = warp_protocol::PrivateKey::random(&mut rand::rng());
        let public_key = private_key.public_key();
        let public_key_string = warp_protocol::crypto::pubkey_to_string(&public_key);

        if re.is_match(&public_key_string) {
            println!(
                "Private key: {}",
                warp_protocol::crypto::privkey_to_string(&private_key)
            );
            println!("Public key: {}", public_key_string);
            break;
        }
    }

    Ok(())
}
