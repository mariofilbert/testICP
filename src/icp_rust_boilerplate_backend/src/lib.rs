#[macro_use]
extern crate serde;

use candid::{Decode, Encode};
use ic_cdk::api::time;
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, DefaultMemoryImpl, StableBTreeMap, Storable};
use std::{borrow::Cow, cell::RefCell};
use std::collections::HashSet; // Import HashSet for managing IDs

// Define the type of memory to be used
type Memory = VirtualMemory<DefaultMemoryImpl>;

// Struct representing a Warehouse
#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct Warehouse {
    id: u64,
    name: String,
    created_at: u64,
}

// Struct representing a StockItem
#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct StockItem {
    item_id: u64,
    warehouse_id: u64,
    item_name: String,
    quantity: u64,
    created_at: u64,
    updated_at: Option<u64>,
}

// Implementations for serialization and deserialization of Warehouse
impl Storable for Warehouse {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

// Bounded storage for Warehouse to limit size
impl BoundedStorable for Warehouse {
    const MAX_SIZE: u32 = 1024; // Maximum size in bytes
    const IS_FIXED_SIZE: bool = false; // Not a fixed size
}

// Implementations for serialization and deserialization of StockItem
impl Storable for StockItem {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

// Bounded storage for StockItem
impl BoundedStorable for StockItem {
    const MAX_SIZE: u32 = 1024; // Maximum size in bytes
    const IS_FIXED_SIZE: bool = false; // Not a fixed size
}

// Thread-local storage for memory management and counters
thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(
        MemoryManager::init(DefaultMemoryImpl::default())
    );

    static WAREHOUSE_ID_COUNTER: RefCell<HashSet<u64>> = RefCell::new(HashSet::new()); // Store deleted IDs for reuse
    static WAREHOUSE_ID_INCREMENT: RefCell<u64> = RefCell::new(1);  // Counter for new warehouse IDs

    static ITEM_ID_COUNTER: RefCell<Vec<u64>> = RefCell::new(Vec::new()); // Store reusable stock item IDs
    static ITEM_ID_INCREMENT: RefCell<u64> = RefCell::new(1);  // Counter for new stock item IDs

    static WAREHOUSE_STORAGE: RefCell<StableBTreeMap<u64, Warehouse, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))
    ));

    static STOCK_STORAGE: RefCell<StableBTreeMap<u64, StockItem, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))
    ));
}

// Payload structure for adding a Warehouse
#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct WarehousePayload {
    name: String,
}

// Payload structure for adding a StockItem
#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct StockItemPayload {
    warehouse_id: u64,
    item_name: String,
    quantity: u64,
}

// Validate warehouse payload for required fields
fn validate_warehouse_payload(payload: &WarehousePayload) -> Result<(), Error> {
    if payload.name.is_empty() {
        return Err(Error::InvalidInput {
            msg: "Warehouse name cannot be empty".to_string(),
        });
    }
    Ok(())
}

// Validate stock item payload for required fields
fn validate_stock_item_payload(payload: &StockItemPayload) -> Result<(), Error> {
    if payload.item_name.is_empty() {
        return Err(Error::InvalidInput {
            msg: "Item name cannot be empty".to_string(),
        });
    }
    if payload.quantity == 0 {
        return Err(Error::InvalidInput {
            msg: "Quantity must be greater than zero".to_string(),
        });
    }
    Ok(())
}

// Function to get the next available warehouse ID
fn get_next_warehouse_id() -> u64 {
    // First, try to find the smallest reusable ID
    let reusable_id = WAREHOUSE_ID_COUNTER.with(|counter| {
        let counter_ref = counter.borrow(); // Immutable borrow
        counter_ref.iter().min().copied() // Get the smallest available ID
    });

    // If a reusable ID exists, remove it from the set and return it
    if let Some(id) = reusable_id {
        WAREHOUSE_ID_COUNTER.with(|counter| {
            let mut counter_mut = counter.borrow_mut(); // Mutable borrow
            counter_mut.remove(&id); // Remove the reused ID
        });
        return id; // Return the reusable ID
    }

    // If no reusable ID exists, increment the main counter
    WAREHOUSE_ID_INCREMENT.with(|counter| {
        let mut id_counter = counter.borrow_mut(); // Mutable borrow
        let next_id = *id_counter; // Get the current ID
        *id_counter += 1; // Increment for next use
        next_id // Return the current ID
    })
}

// Function to get the next available stock item ID, allowing ID reuse
fn get_next_item_id() -> u64 {
    // Check if there are any reusable IDs in the ITEM_ID_COUNTER
    if let Some(reused_id) = ITEM_ID_COUNTER.with(|counter| {
        let mut counter = counter.borrow_mut();
        if !counter.is_empty() {
            return counter.pop(); // Return the last reusable ID
        }
        None // No reusable ID available
    }) {
        return reused_id; // Return the reused ID if available
    }

    // If no reusable IDs are available, increment the counter for new IDs starting from 1
    ITEM_ID_INCREMENT.with(|counter| {
        let mut id = counter.borrow_mut();
        let next_id = *id;   // Get the current ID
        *id += 1;            // Increment for next use
        next_id              // Return the current ID
    })
}

// Function to retrieve a warehouse by ID
#[ic_cdk::query]
fn get_warehouse(id: u64) -> Result<Warehouse, Error> {
    match _get_warehouse(&id) {
        Some(warehouse) => Ok(warehouse),
        None => Err(Error::NotFound {
            msg: format!("A warehouse with id={} not found", id),
        }),
    }
}

// Function to add a new warehouse
#[ic_cdk::update]
fn add_warehouse(payload: WarehousePayload) -> Result<Warehouse, Error> {
    // Validate the incoming payload
    validate_warehouse_payload(&payload)?;

    let id = get_next_warehouse_id();  // Get the next available ID

    let warehouse = Warehouse {
        id,
        name: payload.name,
        created_at: time(),
    };

    // Insert the new warehouse into storage
    WAREHOUSE_STORAGE.with(|storage| {
        storage.borrow_mut().insert(id, warehouse.clone());
    });

    Ok(warehouse) // Return the newly created warehouse
}

// Function to delete a warehouse by ID
#[ic_cdk::update]
fn delete_warehouse(warehouse_id: u64) -> Result<(), Error> {
    // Step 1: Remove the warehouse and check if it was found
    let warehouse_found = WAREHOUSE_STORAGE.with(|storage| {
        let mut storage_mut = storage.borrow_mut(); // Get mutable borrow
        storage_mut.remove(&warehouse_id).is_some() // Remove and check if it existed
    });

    if !warehouse_found {
        return Err(Error::NotFound {
            msg: format!("Warehouse with id={} not found", warehouse_id),
        });
    }

    // Add the deleted ID to the HashSet for reuse
    WAREHOUSE_ID_COUNTER.with(|counter| {
        let mut counter_mut = counter.borrow_mut();
        counter_mut.insert(warehouse_id); // Add to reusable ID set
    });

    // Step 2: Now delete all stock items associated with the warehouse
    STOCK_STORAGE.with(|storage| {
        let mut stock_storage = storage.borrow_mut(); // Get mutable borrow

        // Collect stock item IDs to remove
        let item_ids_to_remove: Vec<u64> = stock_storage.iter()
            .filter(|(_, item)| item.warehouse_id == warehouse_id)
            .map(|(id, _)| id)
            .collect();

        // Remove the stock items
        for item_id in item_ids_to_remove {
            stock_storage.remove(&item_id);
        }
    });

    Ok(()) // Return success
}

// Function to get all warehouses along with their associated stock items
#[ic_cdk::query]
fn get_all_warehouses_with_stocks() -> Vec<(Warehouse, Vec<StockItem>)> {
    let mut result = Vec::new();

    WAREHOUSE_STORAGE.with(|warehouse_storage| {
        let storage_ref = warehouse_storage.borrow(); // Immutable borrow
        for (_, warehouse) in storage_ref.iter() {
            let items = STOCK_STORAGE.with(|stock_storage| {
                let stock_ref = stock_storage.borrow(); // Immutable borrow
                stock_ref.iter()
                    .filter(|(_, item)| item.warehouse_id == warehouse.id)
                    .map(|(_, item)| item.clone())
                    .collect::<Vec<_>>() // Collect associated stock items
            });
            result.push((warehouse.clone(), items)); // Add to results
        }
    });

    result // Return the accumulated warehouses and their items
}

// Function to add a stock item
#[ic_cdk::update]
fn add_stock_item(payload: StockItemPayload) -> Result<StockItem, Error> {
    // Validate the incoming payload
    validate_stock_item_payload(&payload)?;

    let id = get_next_item_id(); // Get the next available item ID

    let stock_item = StockItem {
        item_id: id,
        warehouse_id: payload.warehouse_id,
        item_name: payload.item_name,
        quantity: payload.quantity,
        created_at: time(),
        updated_at: None,
    };

    // Insert the new stock item into storage
    STOCK_STORAGE.with(|storage| {
        storage.borrow_mut().insert(id, stock_item.clone());
    });

    Ok(stock_item) // Return the newly created stock item
}

// Function to delete a stock item by ID
#[ic_cdk::update]
fn delete_stock_item(item_id: u64) -> Result<(), Error> {
    let item_found = STOCK_STORAGE.with(|storage| {
        let mut storage_mut = storage.borrow_mut(); // Get mutable borrow
        storage_mut.remove(&item_id).is_some() // Remove and check if it existed
    });

    if !item_found {
        return Err(Error::NotFound {
            msg: format!("Stock item with id={} not found", item_id),
        });
    }

    // Add the deleted ID to the item ID counter for reuse
    ITEM_ID_COUNTER.with(|counter| {
        let mut counter_mut = counter.borrow_mut();
        counter_mut.push(item_id); // Store the ID for future reuse
    });

    Ok(()) // Return success
}

// Define error types for handling various error states
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Error {
    InvalidInput { msg: String },
    NotFound { msg: String },
}

// Main function to initialize the program
#[ic_cdk::main]
fn main() {
    // Initialization logic if needed
}
