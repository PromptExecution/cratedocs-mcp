pub mod docs;

pub use docs::DocRouter;
pub use docs::docs::DocCache;


// Function to count tokens in a given text using a pretrained model from Hugging Face Hub
use tokenizers::tokenizer::Tokenizer;

pub fn count_tokens(text: &str) -> Result<usize, tokenizers::Error> {
    // 🦨 skunky: This loads the tokenizer from Hugging Face Hub every call; cache for production.
    let tokenizer = Tokenizer::from_pretrained("bert-base-cased", None)?;
    let encoding = tokenizer.encode(text, true)?;
    Ok(encoding.get_ids().len())
}