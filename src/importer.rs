// src/importer.rs

#![cfg(feature = "csv_import")]

use crate::pb::{Card, Deck};
use std::path::Path;

/// Reads a standard two-column CSV and converts it into a Protobuf Deck
pub fn from_csv(file_path: &Path) -> Result<Deck, Box<dyn std::error::Error>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false) 
        .from_path(file_path)?;

    let mut deck = Deck::default();
    deck.title = "Imported Deck".to_string();

    for (index, result) in reader.records().enumerate() {
        let record = result?;
        
        if record.len() >= 2 {
            let mut card = Card::default();
            card.id = format!("imported_card_{}", index);
            card.kind = 1; // Basic Card
            card.term = Some(record[0].to_string());
            card.definition = Some(record[1].to_string());
            
            deck.cards.push(card);
        }
    }

    Ok(deck)
}
