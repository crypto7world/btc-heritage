use serde::{de::DeserializeOwned, Serialize};

use super::Database;
use crate::errors::{Error, Result};

pub trait DatabaseItem: Serialize + DeserializeOwned {
    fn item_key_prefix() -> &'static str;
    fn item_default_name_key_prefix() -> &'static str;
    fn name(&self) -> &str;
    fn rename(&mut self, new_name: String);

    // Blanket implementations
    fn name_to_key(name: &str) -> String {
        format!("{}{name}", Self::item_key_prefix())
    }

    fn list_names(db: &Database) -> Result<Vec<String>> {
        let keys_with_prefix = db.list_keys(Some(Self::item_key_prefix()))?;
        Ok(keys_with_prefix
            .into_iter()
            .map(|k| {
                k.strip_prefix(Self::item_key_prefix())
                    .expect("we asked for keys with this prefix")
                    .to_owned()
            })
            .collect())
    }

    fn all_in_db(db: &Database) -> Result<Vec<Self>> {
        db.query(Self::item_key_prefix())
    }

    /// Get the default name of the item
    /// It is "default" by default but can be changed by invoking [DatabaseItem::set_default_wallet_name]
    fn get_default_item_name(db: &Database) -> Result<String> {
        Ok(db
            .get_item(Self::item_default_name_key_prefix())?
            .unwrap_or_else(|| "default".to_owned()))
    }
    /// Set the default name of the item
    fn set_default_item_name(db: &mut Database, name: String) -> Result<()> {
        db.update_item(Self::item_default_name_key_prefix(), &name)?;
        Ok(())
    }

    /// Verify that the given item name is not already in the database
    fn verify_name_is_free(db: &Database, name: &str) -> Result<()> {
        if db.contains_key(&Self::name_to_key(name))? {
            Err(Error::ItemAlreadyExist(name.to_owned()))
        } else {
            Ok(())
        }
    }

    fn create(&self, db: &mut Database) -> Result<()> {
        db.put_item(&Self::name_to_key(self.name()), self)?;
        Ok(())
    }

    fn delete(&self, db: &mut Database) -> Result<()> {
        db.delete_item::<Self>(&Self::name_to_key(self.name()))?;
        Ok(())
    }

    fn save(&self, db: &mut Database) -> Result<()> {
        db.update_item(&Self::name_to_key(&self.name()), self)?;
        Ok(())
    }

    fn load(db: &Database, name: &str) -> Result<Self> {
        db.get_item(&Self::name_to_key(name))?
            .ok_or(Error::InexistantItem(name.to_owned()))
    }

    fn db_rename(&mut self, db: &mut Database, new_name: String) -> Result<()> {
        let old_name = self.name().to_owned();
        self.rename(new_name);
        db.put_item(&Self::name_to_key(&self.name()), self)?;
        db.delete_item::<Self>(&Self::name_to_key(&old_name))?;
        Ok(())
    }
}

macro_rules! impl_db_item {
    ($name:ident, $key_pref:literal, $default_name_key:literal $($code:tt)* ) => {
        impl $name {
            const DB_KEY_PREFIX: &'static str = $key_pref;
            const DEFAULT_NAME_KEY: &'static str = $default_name_key;
        }
        impl DatabaseItem for $name {
            fn item_key_prefix() -> &'static str {
                Self::DB_KEY_PREFIX
            }

            fn item_default_name_key_prefix() -> &'static str {
                Self::DEFAULT_NAME_KEY
            }

            fn name(&self) -> &str {
                &self.name
            }

            fn rename(&mut self, new_name: String) {
                self.name = new_name;
            }

            $($code)*
        }
    };
}
pub(crate) use impl_db_item;
