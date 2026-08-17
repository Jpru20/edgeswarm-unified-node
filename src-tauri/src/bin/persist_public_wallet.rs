use edgeswarm_unified_node_lib::core::wallet_public_identity::WalletPublicIdentity;
fn main(){let a=std::env::var("EDGESWARM_WALLET_ADDRESS").expect("wallet address missing");let w=WalletPublicIdentity::save_current(&a).expect("wallet identity save failed");println!("WALLET_PUBLIC_IDENTITY_SAVED=true");println!("HARDWARE_ID={}",w.hardware_id);println!("PRIVATE_KEY_STORED=false");}
