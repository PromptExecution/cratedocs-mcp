pub mod docs;

pub use docs::DocRouter;
pub use docs::docs::DocCache;

use tokenizers::Tokenizer;
use tokenizers::models::wordpiece::WordPiece;

// Function to count tokens in a given text
pub fn count_tokens(text: &str) -> Result<usize, tokenizers::Error> {
    // NOTE: You must provide a valid vocab file path for WordPiece
    let model = WordPiece::from_file("path/to/vocab.txt").build()?;
    let tokenizer = Tokenizer::new(model);
    let tokens = tokenizer.encode(text, true)?;
    Ok(tokens.get_ids().len())
}