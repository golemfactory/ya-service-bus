#[cfg(feature = "with-bytes")]
pub mod bytes;

#[cfg(feature = "with-futures")]
pub mod futures;

#[cfg(feature = "with-futures")]
pub use crate::futures::IntoFlatten as _;

use std::collections::{hash_map::Entry, HashMap};

struct RevPrefixes<'a>(&'a str);

impl<'a> Iterator for RevPrefixes<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        let cv = self.0;
        if cv.is_empty() {
            return None;
        }
        if let Some(sep_pos) = cv.rfind("/") {
            self.0 = &cv[..sep_pos];
        } else {
            self.0 = ""
        }
        return Some(cv);
    }
}

pub struct PrefixLookupBag<T> {
    dict: HashMap<String, T>,
}

impl<T> Default for PrefixLookupBag<T> {
    fn default() -> Self {
        PrefixLookupBag {
            dict: HashMap::new(),
        }
    }
}

impl<T> PrefixLookupBag<T> {

    pub fn get(&self, key: &str) -> Option<&T> {
        RevPrefixes(key).find_map(|key| self.dict.get(key))
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.dict.keys()
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut T> {
        if let Some(k) = RevPrefixes(key).find(|&k| self.dict.contains_key(k)) {
            self.dict.get_mut(k)
        } else {
            None
        }
    }

    pub fn insert(&mut self, key: String, v: T) -> Option<T> {
        self.dict.insert(key, v)
    }

    pub fn entry(&mut self, key: String) -> Entry<String, T> {
        if let Some(k) = RevPrefixes(&key).find(|&k| self.dict.contains_key(k)) {
            self.dict.entry(k.to_owned())
        } else {
            self.dict.entry(key)
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<T> {
        self.dict.remove(key)
    }

    pub fn len(&self) -> usize{
        self.dict.len()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_rev() {
        let v: Vec<_> = RevPrefixes("/net/0x1212xx/from/0x123/exeunit/exec").collect();
        assert_eq!(
            v,
            [
                "/net/0x1212xx/from/0x123/exeunit/exec",
                "/net/0x1212xx/from/0x123/exeunit",
                "/net/0x1212xx/from/0x123",
                "/net/0x1212xx/from",
                "/net/0x1212xx",
                "/net"
            ]
        );
        let v: Vec<_> = RevPrefixes("").collect();
        assert!(v.is_empty());
    }

    #[test]
    fn test_prefix_bag() {
        let mut bag = PrefixLookupBag::default();

        bag.insert("/ala/ma/kota".into(), 1);
        bag.insert("/ala/ma/psa".into(), 2);
        bag.insert("/ala".into(), 7);

        assert_eq!(bag.get("/ala/ma/smoka"), Some(&7));
        assert_eq!(bag.get("/ala/ma/kota"), Some(&1));
        assert_eq!(bag.get("/jola/ma/psa"), None);
    }
}
