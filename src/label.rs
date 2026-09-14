#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelAlphabet {
    symbols: Vec<char>,
}

impl LabelAlphabet {
    pub fn new(symbols: &str) -> Result<Self, String> {
        let symbols: Vec<char> = symbols.chars().collect();
        if symbols.len() < 2 || symbols.len() > 26 {
            return Err("must contain 2-26 symbols".to_owned());
        }
        Ok(Self { symbols })
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    pub fn symbol(&self, index: usize) -> char {
        self.symbols[index]
    }

    pub fn find(&self, symbol: char) -> Option<usize> {
        self.symbols
            .iter()
            .position(|candidate| *candidate == symbol)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppendResult {
    Success,
    Full,
    Overflow,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelSelection {
    alphabet: LabelAlphabet,
    num_labels: usize,
    len: usize,
    input: Vec<usize>,
}

impl LabelSelection {
    pub fn new(alphabet: LabelAlphabet, num_labels: usize) -> Self {
        let mut remaining = num_labels;
        let mut len = 0;
        while remaining > 0 {
            len += 1;
            remaining /= alphabet.symbols.len();
        }
        Self {
            alphabet,
            num_labels,
            len,
            input: Vec::with_capacity(len),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[cfg(test)]
    pub fn clear(&mut self) {
        self.input.clear();
    }

    pub fn append(&mut self, index: usize) -> AppendResult {
        if self.input.len() >= self.len {
            return AppendResult::Full;
        }
        self.input.push(index);
        if self.partial_index() >= self.num_labels {
            self.input.pop();
            return AppendResult::Overflow;
        }
        AppendResult::Success
    }

    pub fn backspace(&mut self) -> bool {
        self.input.pop().is_some()
    }

    pub fn partial_index(&self) -> usize {
        let mut index = 0;
        let mut factor = 1;
        for digit in &self.input {
            index += digit * factor;
            factor *= self.alphabet.symbols.len();
        }
        index
    }

    pub fn resolve(&self) -> Option<usize> {
        (self.input.len() == self.len).then(|| self.partial_index())
    }

    pub fn label_for_index(&self, mut index: usize) -> Option<String> {
        let base = self.alphabet.symbols.len();
        let mut digits = Vec::with_capacity(self.len);
        for _ in 0..self.len {
            digits.push(index % base);
            index /= base;
        }
        (index == 0).then(|| {
            digits
                .into_iter()
                .map(|digit| self.alphabet.symbol(digit))
                .collect()
        })
    }

    pub fn matches_index(&self, index: usize) -> bool {
        let Some(label) = self.label_for_index(index) else {
            return false;
        };
        label
            .chars()
            .zip(self.input.iter().copied())
            .take(self.input.len())
            .all(|(symbol, expected)| self.alphabet.find(symbol) == Some(expected))
    }

    pub fn split_index(&self, index: usize) -> Option<(String, String)> {
        let label = self.label_for_index(index)?;
        let cut = self.input.len().min(self.len);
        let mut chars = label.chars();
        let prefix: String = chars.by_ref().take(cut).collect();
        let suffix: String = chars.collect();
        Some((prefix, suffix))
    }

    #[cfg(test)]
    pub fn typed_string(&self) -> String {
        self.input
            .iter()
            .map(|digit| self.alphabet.symbol(*digit))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alphabet() -> LabelAlphabet {
        LabelAlphabet::new("abcd").unwrap()
    }

    #[test]
    fn fixed_length_matches_wl_kbptr_scheme() {
        let selection = LabelSelection::new(alphabet(), 17);
        assert_eq!(selection.len(), 3);
        assert_eq!(selection.label_for_index(0).unwrap(), "aaa");
        assert_eq!(selection.label_for_index(1).unwrap(), "baa");
        assert_eq!(selection.label_for_index(4).unwrap(), "aba");
        assert_eq!(selection.label_for_index(16).unwrap(), "aab");
        assert!(selection.label_for_index(64).is_none());
    }

    #[test]
    fn append_backspace_match_and_resolve_follow_prefix_rules() {
        let mut selection = LabelSelection::new(alphabet(), 10);
        assert_eq!(selection.append(1), AppendResult::Success);
        assert_eq!(selection.typed_string(), "b");
        assert!(selection.matches_index(1));
        assert!(selection.matches_index(5));
        assert!(!selection.matches_index(0));
        assert_eq!(selection.append(2), AppendResult::Success);
        assert_eq!(selection.typed_string(), "bc");
        assert_eq!(selection.resolve().unwrap(), 9);
        assert_eq!(
            selection.split_index(9).unwrap(),
            ("bc".to_owned(), String::new())
        );
        assert!(selection.backspace());
        assert_eq!(selection.typed_string(), "b");
        assert!(selection.backspace());
        assert!(!selection.backspace());
        assert_eq!(selection.typed_string(), "");
    }

    #[test]
    fn append_rejects_overflow_and_full_inputs() {
        let mut selection = LabelSelection::new(alphabet(), 6);
        assert_eq!(selection.len(), 2);
        assert_eq!(selection.append(1), AppendResult::Success);
        assert_eq!(selection.append(1), AppendResult::Success);
        assert_eq!(selection.resolve(), Some(5));
        selection.clear();
        assert_eq!(selection.append(2), AppendResult::Success);
        assert_eq!(selection.append(1), AppendResult::Overflow);
        assert_eq!(selection.typed_string(), "c");
        let mut full = LabelSelection::new(alphabet(), 1);
        assert_eq!(full.append(0), AppendResult::Success);
        assert_eq!(full.append(0), AppendResult::Full);
    }

    #[test]
    fn symbol_lookup_supports_subsets() {
        let alphabet = LabelAlphabet::new("hjkl").unwrap();
        assert_eq!(alphabet.find('h'), Some(0));
        assert_eq!(alphabet.find('k'), Some(2));
        assert_eq!(alphabet.find('a'), None);
    }
}
