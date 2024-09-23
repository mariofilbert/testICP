#[macro_use]
extern crate serde;
use candid::{Decode, Encode};
use ic_cdk::api::time;
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, DefaultMemoryImpl, StableBTreeMap, Storable};
use std::{borrow::Cow, cell::RefCell};

type Memory = VirtualMemory<DefaultMemoryImpl>;

#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct Warehouse {
    id: u64,
    name: String,
    created_at: u64,
}

#[derive(candid::CandidType, Clone, Serialize, Deserialize, Default)]
struct StockItem {
    item_id: u64,
    warehouse_id: u64,
    item_name: String,
    quantity: u64,
    created_at: u64,
    updated_at: Option<u64>,
}

impl Storable for Warehouse {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for Warehouse {
    const MAX_SIZE: u32 = 1024;
    const IS_FIXED_SIZE: bool = false;
}

impl Storable for StockItem {
    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }

    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }
}

impl BoundedStorable for StockItem {
    const MAX_SIZE: u32 = 1024;
    const IS_FIXED_SIZE: bool = false;
}

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(
        MemoryManager::init(DefaultMemoryImpl::default())
    );

    static WAREHOUSE_ID_COUNTER: RefCell<Vec<u64>> = RefCell::new(Vec::new());
    static ITEM_ID_COUNTER: RefCell<Vec<u64>> = RefCell::new(Vec::new());

    static WAREHOUSE_STORAGE: RefCell<StableBTreeMap<u64, Warehouse, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(2)))
    ));

    static STOCK_STORAGE: RefCell<StableBTreeMap<u64, StockItem, Memory>> =
        RefCell::new(StableBTreeMap::init(
            MEMORY_MANAGER.with(|m| m.borrow().get(MemoryId::new(3)))
    ));
}

#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct WarehousePayload {
    name: String,
}

#[derive(candid::CandidType, Serialize, Deserialize, Default)]
struct StockItemPayload {
    warehouse_id: u64,
    item_name: String,
    quantity: u64,
}

// Function to get the next available ID
fn get_next_available_id(counter: &mut Vec<u64>) -> u64 {
    if let Some(id) = counter.pop() {
        id
    } else {
        // If no IDs are available, return the next highest ID
        let next_id = counter.len() as u64;
        next_id
    }
}


#[ic_cdk::query]
fn get_warehouse(id: u64) -> Result<Warehouse, Error> {
    match _get_warehouse(&id) {
        Some(warehouse) => Ok(warehouse),
        None => Err(Error::NotFound {
            msg: format!("A warehouse with id={} not found", id),
        }),
    }
}

#[ic_cdk::update]
fn add_warehouse(payload: WarehousePayload) -> Result<Warehouse, Error> {
    let id = WAREHOUSE_ID_COUNTER.with(|c| {
        let mut counter = c.borrow_mut(); // Keep everything within this closure
        get_next_available_id(&mut counter) // Pass the mutable reference
    });

    let warehouse = Warehouse {
        id,
        name: payload.name,
        created_at: time(),
    };

    WAREHOUSE_STORAGE.with(|storage| {
        storage.borrow_mut().insert(id, warehouse.clone());
    });

    Ok(warehouse)
}

#[ic_cdk::update]
fn delete_warehouse(warehouse_id: u64) -> Result<Warehouse, Error> {
    WAREHOUSE_STORAGE.with(|warehouse_storage| {
        let mut warehouse_store = warehouse_storage.borrow_mut();
        
        if let Some(warehouse) = warehouse_store.remove(&warehouse_id) {
            // Remove items from STOCK_STORAGE
            let items_to_delete: Vec<u64> = STOCK_STORAGE.with(|stock_storage| {
                let  stock_store = stock_storage.borrow_mut();

                stock_store.iter()
                    .filter_map(|(item_id, item)| {
                        if item.warehouse_id == warehouse_id {
                            Some(item_id)
                        } else {
                            None
                        }
                    })
                    .collect()
            });

            for item_id in items_to_delete {
                STOCK_STORAGE.with(|stock_storage| {
                    let mut stock_store = stock_storage.borrow_mut();

                    stock_store.remove(&item_id);
                    ITEM_ID_COUNTER.with(|counter| counter.borrow_mut().push(item_id)); // Reuse the ID
                });
            }

            Ok(warehouse)
        } else {
            Err(Error::NotFound {
                msg: format!("Warehouse with id={} not found", warehouse_id),
            })
        }
    })
}

#[ic_cdk::query]
fn get_all_warehouses_with_stocks() -> Vec<(Warehouse, Vec<StockItem>)> {
    let mut result = Vec::new();

    WAREHOUSE_STORAGE.with(|warehouse_storage| {
        let warehouses = warehouse_storage.borrow();
        for (warehouse_id, warehouse) in warehouses.iter() {
            let stocks: Vec<StockItem> = STOCK_STORAGE.with(|stock_storage| {
                stock_storage.borrow()
                    .iter()
                    .filter_map(|(_, item)| {
                        if item.warehouse_id == warehouse_id {
                            Some(item.clone())
                        } else {
                            None
                        }
                    })
                    .collect()
            });
            result.push((warehouse.clone(), stocks));
        }
    });

    result
}

#[ic_cdk::update]
fn add_item_to_warehouse(payload: StockItemPayload) -> Result<StockItem, Error> {
    // Check if the warehouse exists
    let warehouse_exists = WAREHOUSE_STORAGE.with(|storage| {
        storage.borrow().get(&payload.warehouse_id).is_some()
    });

    if !warehouse_exists {
        return Err(Error::NotFound {
            msg: format!("Warehouse with id={} not found", payload.warehouse_id),
        });
    }

    // Get the next available item ID within the closure
    let item_id = ITEM_ID_COUNTER.with(|c| {
        let mut counter = c.borrow_mut();
        get_next_available_id(&mut counter)
    });

    let item = StockItem {
        item_id,
        warehouse_id: payload.warehouse_id,
        item_name: payload.item_name,
        quantity: payload.quantity,
        created_at: time(),
        updated_at: None,
    };

    STOCK_STORAGE.with(|storage| {
        storage.borrow_mut().insert(item_id, item.clone());
    });

    Ok(item)
}


#[ic_cdk::query]
fn check_stock(item_id: u64) -> Result<StockItem, Error> {
    match STOCK_STORAGE.with(|storage| storage.borrow().get(&item_id)) {
        Some(stock_item) => Ok(stock_item.clone()), // Return a clone
        None => Err(Error::NotFound {
            msg: format!("Item with id={} not found", item_id),
        }),
    }
}

#[ic_cdk::update]
fn delete_item(item_id: u64) -> Result<StockItem, Error> {
    STOCK_STORAGE.with(|storage| {
        let mut stock = storage.borrow_mut();

        
        if let Some(item) = stock.remove(&item_id) {
            ITEM_ID_COUNTER.with(|counter| counter.borrow_mut().push(item_id)); // Reuse the ID
            Ok(item)
        } else {
            Err(Error::NotFound {
                msg: format!("Item with id={} not found", item_id),
            })
        }
    })
}

#[ic_cdk::update]
fn transfer_item(item_id: u64, from_warehouse_id: u64, to_warehouse_id: u64, quantity: u64) -> Result<(), Error> {
    STOCK_STORAGE.with(|storage| {
        let mut stock = storage.borrow_mut();
        
        if let Some(mut item) = stock.remove(&item_id) {
            if item.warehouse_id != from_warehouse_id {
                stock.insert(item_id, item.clone());
                return Err(Error::NotFound {
                    msg: format!(
                        "Item with id={} not found in warehouse_id={}",
                        item_id, from_warehouse_id
                    ),
                });
            }

            if item.quantity < quantity {
                stock.insert(item_id, item.clone());
                return Err(Error::NotEnoughStock {
                    msg: format!(
                        "Not enough stock for item_id={}, available={}, requested={}",
                        item_id, item.quantity, quantity
                    ),
                });
            }

            item.quantity -= quantity;
            item.updated_at = Some(time());

            stock.insert(item_id, item.clone());

            // Create a new item record for the destination warehouse
            let new_item = StockItem {
                item_id: ITEM_ID_COUNTER.with(|c| {
                    let mut counter = c.borrow_mut();
                    get_next_available_id(&mut counter)
                }),
                warehouse_id: to_warehouse_id,
                item_name: item.item_name.clone(),
                quantity,
                created_at: time(),
                updated_at: None,
            };

            stock.insert(new_item.item_id, new_item);
            
            Ok(())
        } else {
            Err(Error::NotFound {
                msg: format!("Item with id={} not found", item_id),
            })
        }
    })
}


#[ic_cdk::query]
fn get_warehouse_stock(warehouse_id: u64) -> Vec<StockItem> {
    STOCK_STORAGE.with(|storage| {
        storage
            .borrow()
            .iter()
            .filter_map(|(_, item)| {
                if item.warehouse_id == warehouse_id {
                    Some(item.clone()) // Return a clone
                } else {
                    None
                }
            })
            .collect()
    })
}

#[derive(candid::CandidType, Deserialize, Serialize)]
enum Error {
    NotFound { msg: String },
    NotEnoughStock { msg: String },
}

// Helper functions
fn _get_warehouse(id: &u64) -> Option<Warehouse> {
    WAREHOUSE_STORAGE.with(|service| service.borrow().get(id))
}

// need this to generate candid
ic_cdk::export_candid!();
