// Copyright 2016 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

use core::{CLIENT_STRUCTURED_DATA_TAG, immut_data_operations};
use core::client::Client;
use core::errors::CoreError;
use core::structured_data_operations::{unversioned, versioned};
use ffi::app::App;
use ffi::errors::FfiError;
use ffi::helper;
use ffi::low_level_api::{CipherOptHandle, DataIdHandle, StructDataHandle};
use ffi::low_level_api::cipher_opt::CipherOpt;
use ffi::low_level_api::object_cache::object_cache;
use maidsafe_utilities::serialisation::{deserialise, serialise};
use routing::{Data, DataIdentifier, ImmutableData, StructuredData, XOR_NAME_LEN, XorName};
use std::{mem, ptr, slice};
use std::sync::{Arc, Mutex};

/// Create new StructuredData
#[no_mangle]
pub unsafe extern "C" fn struct_data_new(app: *const App,
                                         type_tag: u64,
                                         id: *const [u8; XOR_NAME_LEN],
                                         cipher_opt_h: CipherOptHandle,
                                         data: *const u8,
                                         size: usize,
                                         o_handle: *mut StructDataHandle)
                                         -> i32 {
    helper::catch_unwind_i32(|| {
        let app = &*app;
        let client = app.get_client();
        let xor_id = XorName(*id);
        let plain_text = slice::from_raw_parts(data, size).to_owned();

        let (owner_keys, sign_key) = {
            let client_guard = unwrap!(client.lock());
            let owner_keys = vec![*ffi_try!(client_guard.get_public_signing_key())];
            let sign_key = ffi_try!(client_guard.get_secret_signing_key()).clone();
            (owner_keys, sign_key)
        };

        let sd = match type_tag {
            ::UNVERSIONED_STRUCT_DATA_TYPE_TAG => {
                let raw_data = ffi_try!(ffi_try!(unwrap!(object_cache().lock())
                        .cipher_opt
                        .get_mut(&cipher_opt_h)
                        .ok_or(FfiError::InvalidCipherOptHandle))
                    .encrypt(app, &plain_text));

                ffi_try!(unversioned::create(client,
                                             type_tag,
                                             xor_id,
                                             0,
                                             raw_data,
                                             owner_keys,
                                             Vec::new(),
                                             &sign_key,
                                             None))
            }
            ::VERSIONED_STRUCT_DATA_TYPE_TAG => {
                let immut_data =
                    ffi_try!(immut_data_operations::create(client.clone(), plain_text, None));
                // TODO The above data could be exactly 1 MiB and ideally should not be touched any
                // more. For this however we will require CipherOpt to be in core module. Until
                // that we need to live with this.
                let ser_immut_data = ffi_try!(serialise(&immut_data).map_err(FfiError::from));
                let raw_data = ffi_try!(ffi_try!(unwrap!(object_cache().lock())
                        .cipher_opt
                        .get_mut(&cipher_opt_h)
                        .ok_or(FfiError::InvalidCipherOptHandle))
                    .encrypt(app, &ser_immut_data));

                let immut_data_final = Data::Immutable(ImmutableData::new(raw_data));
                let immut_data_final_name = *immut_data_final.name();

                let resp_getter = ffi_try!(unwrap!(client.lock()).put(immut_data_final, None));
                ffi_try!(resp_getter.get());

                ffi_try!(versioned::create(client,
                                           immut_data_final_name,
                                           type_tag,
                                           xor_id,
                                           0,
                                           owner_keys,
                                           Vec::new(),
                                           &sign_key))
            }
            x if x >= CLIENT_STRUCTURED_DATA_TAG => {
                let raw_data = ffi_try!(ffi_try!(unwrap!(object_cache().lock())
                        .cipher_opt
                        .get_mut(&cipher_opt_h)
                        .ok_or(FfiError::InvalidCipherOptHandle))
                    .encrypt(app, &plain_text));

                ffi_try!(StructuredData::new(type_tag,
                                             xor_id,
                                             0,
                                             raw_data,
                                             owner_keys,
                                             Vec::new(),
                                             Some(&sign_key))
                    .map_err(CoreError::from))
            }
            _ => ffi_try!(Err(FfiError::InvalidStructuredDataTypeTag)),
        };


        let mut obj_cache = unwrap!(object_cache().lock());
        let handle = obj_cache.new_handle();
        if let Some(prev) = obj_cache.struct_data.insert(handle, sd) {
            debug!("Displaced StructuredData from ObjectCache: {:?}", prev);
        }
        ptr::write(o_handle, handle);

        0
    })
}

/// Fetch an existing StructuredData from Network.
#[no_mangle]
pub unsafe extern "C" fn struct_data_fetch(app: *const App,
                                           data_id_h: DataIdHandle,
                                           o_handle: *mut StructDataHandle)
                                           -> i32 {
    helper::catch_unwind_i32(|| {
        let client = (*app).get_client();
        let data_id = *ffi_try!(unwrap!(object_cache().lock())
            .data_id
            .get_mut(&data_id_h)
            .ok_or(FfiError::InvalidDataIdHandle));
        let resp_getter = ffi_try!(unwrap!(client.lock()).get(data_id, None));
        let sd = match ffi_try!(resp_getter.get()) {
            Data::Structured(sd) => sd,
            _ => ffi_try!(Err(CoreError::ReceivedUnexpectedData)),
        };

        let mut obj_cache = unwrap!(object_cache().lock());
        let handle = obj_cache.new_handle();
        if let Some(prev) = obj_cache.struct_data.insert(handle, sd) {
            debug!("Displaced StructuredData from ObjectCache: {:?}", prev);

        }
        ptr::write(o_handle, handle);

        0
    })
}

// TODO possibly move this to data_id module
/// Extract DataIdentifier from StructuredData.
#[no_mangle]
pub unsafe extern "C" fn struct_data_extract_data_id(sd_h: StructDataHandle,
                                                     o_handle: *mut DataIdHandle)
                                                     -> i32 {
    helper::catch_unwind_i32(|| {
        let mut obj_cache = unwrap!(object_cache().lock());
        let data_id =
            ffi_try!(obj_cache.struct_data.get_mut(&sd_h).ok_or(FfiError::InvalidStructDataHandle))
                .identifier();
        let handle = obj_cache.new_handle();
        if let Some(prev) = obj_cache.data_id.insert(handle, data_id) {
            debug!("Displaced DataIdentifier from ObjectCache: {:?}", prev);
        }
        ptr::write(o_handle, handle);

        0
    })
}

// TODO See if we can extract common functionality and merge with new() above
/// Put new data into StructuredData. Version is not updated. It will be updated on POST.
#[no_mangle]
pub unsafe extern "C" fn struct_data_new_data(app: *const App,
                                              sd_h: StructDataHandle,
                                              cipher_opt_h: CipherOptHandle,
                                              data: *const u8,
                                              size: usize)
                                              -> i32 {
    helper::catch_unwind_i32(|| {
        let mut sd = ffi_try!(unwrap!(object_cache().lock())
            .struct_data
            .remove(&sd_h)
            .ok_or(FfiError::InvalidStructDataHandle));

        let app = &*app;
        let client = app.get_client();
        let plain_text = slice::from_raw_parts(data, size).to_owned();

        let sign_key = ffi_try!(unwrap!(client.lock()).get_secret_signing_key()).clone();

        sd = match sd.get_type_tag() {
            ::UNVERSIONED_STRUCT_DATA_TYPE_TAG => {
                let raw_data = ffi_try!(ffi_try!(unwrap!(object_cache().lock())
                        .cipher_opt
                        .get_mut(&cipher_opt_h)
                        .ok_or(FfiError::InvalidCipherOptHandle))
                    .encrypt(app, &plain_text));

                ffi_try!(unversioned::create(client,
                                             sd.get_type_tag(),
                                             *sd.name(),
                                             sd.get_version(),
                                             raw_data,
                                             // TODO I am discarding this SD. Why does routing make
                                             // me clone unnecessarily ? - check.
                                             sd.get_owner_keys().clone(),
                                             sd.get_previous_owner_keys().clone(),
                                             &sign_key,
                                             None))
            }
            ::VERSIONED_STRUCT_DATA_TYPE_TAG => {
                let immut_data =
                    ffi_try!(immut_data_operations::create(client.clone(), plain_text, None));
                let ser_immut_data = ffi_try!(serialise(&immut_data).map_err(FfiError::from));
                let raw_data = ffi_try!(ffi_try!(unwrap!(object_cache().lock())
                        .cipher_opt
                        .get_mut(&cipher_opt_h)
                        .ok_or(FfiError::InvalidCipherOptHandle))
                    .encrypt(app, &ser_immut_data));

                let immut_data_final = Data::Immutable(ImmutableData::new(raw_data));
                let immut_data_final_name = *immut_data_final.name();

                let resp_getter = ffi_try!(unwrap!(client.lock()).put(immut_data_final, None));
                ffi_try!(resp_getter.get());

                ffi_try!(versioned::append_version(client, sd, immut_data_final_name, &sign_key))
            }
            x if x >= CLIENT_STRUCTURED_DATA_TAG => {
                let raw_data = ffi_try!(ffi_try!(unwrap!(object_cache().lock())
                        .cipher_opt
                        .get_mut(&cipher_opt_h)
                        .ok_or(FfiError::InvalidCipherOptHandle))
                    .encrypt(app, &plain_text));

                ffi_try!(StructuredData::new(sd.get_type_tag(),
                                             *sd.name(),
                                             sd.get_version(),
                                             raw_data,
                                             sd.get_owner_keys().clone(),
                                             sd.get_previous_owner_keys().clone(),
                                             Some(&sign_key))
                    .map_err(CoreError::from))
            }
            _ => ffi_try!(Err(FfiError::InvalidStructuredDataTypeTag)),
        };

        if let Some(prev) = unwrap!(object_cache().lock()).struct_data.insert(sd_h, sd) {
            debug!("Displaced StructuredData from ObjectCache: {:?}", prev);
        }

        0
    })
}

/// Extract data from StructuredData
#[no_mangle]
pub unsafe extern "C" fn struct_data_extract_data(app: *const App,
                                                  sd_h: StructDataHandle,
                                                  o_data: *mut *mut u8,
                                                  o_size: *mut usize,
                                                  o_capacity: *mut usize)
                                                  -> i32 {
    helper::catch_unwind_i32(|| {
        let app = &*app;
        let client = app.get_client();

        // Note: Order of locking is object_cache followed by client - ensure this order
        // everywhere.
        let mut obj_cache = unwrap!(object_cache().lock());
        let sd =
            ffi_try!(obj_cache.struct_data.get_mut(&sd_h).ok_or(FfiError::InvalidStructDataHandle));

        let mut plain_text = match sd.get_type_tag() {
            ::UNVERSIONED_STRUCT_DATA_TYPE_TAG => {
                let raw_data = ffi_try!(unversioned::get_data(client, sd, None));
                ffi_try!(CipherOpt::decrypt(&app, &raw_data))
            }
            ::VERSIONED_STRUCT_DATA_TYPE_TAG => {
                let mut versions = ffi_try!(versioned::get_all_versions(client.clone(), sd));
                if let Some(immut_data_final_name) = versions.pop() {
                    let resp_getter = ffi_try!(unwrap!(client.lock())
                        .get(DataIdentifier::Immutable(immut_data_final_name), None));
                    let immut_data_final = match ffi_try!(resp_getter.get()) {
                        Data::Immutable(immut_data) => immut_data,
                        _ => ffi_try!(Err(CoreError::ReceivedUnexpectedData)),
                    };

                    let ser_immut_data = ffi_try!(CipherOpt::decrypt(&app,
                                                                     immut_data_final.value()));
                    let immut_data = ffi_try!(deserialise::<ImmutableData>(&ser_immut_data)
                        .map_err(FfiError::from));
                    ffi_try!(immut_data_operations::get_data(client, *immut_data.name(), None))
                } else {
                    Vec::new()
                }
            }
            x if x >= CLIENT_STRUCTURED_DATA_TAG => {
                ffi_try!(CipherOpt::decrypt(&app, sd.get_data()))
            }
            _ => ffi_try!(Err(FfiError::InvalidStructuredDataTypeTag)),
        };

        *o_data = plain_text.as_mut_ptr();
        ptr::write(o_size, plain_text.len());
        ptr::write(o_capacity, plain_text.capacity());
        mem::forget(plain_text);

        0
    })
}

/// Get number of versions from a versioned StructuredData
#[no_mangle]
pub unsafe extern "C" fn struct_data_num_of_versions(app: *const App,
                                                     sd_h: StructDataHandle,
                                                     o_num: *mut usize)
                                                     -> i32 {
    helper::catch_unwind_i32(|| {
        let mut obj_cache = unwrap!(object_cache().lock());
        let sd =
            ffi_try!(obj_cache.struct_data.get_mut(&sd_h).ok_or(FfiError::InvalidStructDataHandle));

        // TODO Move this check to core module itself (from all functions here using this pattern.
        if sd.get_type_tag() != ::VERSIONED_STRUCT_DATA_TYPE_TAG {
            ffi_try!(Err(FfiError::InvalidStructuredDataTypeTag));
        }

        let num = ffi_try!(versioned::get_all_versions((*app).get_client(), sd)).len();

        ptr::write(o_num, num);

        0
    })
}

/// Get nth (starts from 0) version from a versioned StructuredData
#[no_mangle]
pub unsafe extern "C" fn struct_data_nth_version(app: *const App,
                                                 sd_h: StructDataHandle,
                                                 n: usize,
                                                 o_data: *mut *mut u8,
                                                 o_size: *mut usize,
                                                 o_capacity: *mut usize)
                                                 -> i32 {
    helper::catch_unwind_i32(|| {
        let app = &*app;
        let client = app.get_client();

        let mut versions = {
            let mut obj_cache = unwrap!(object_cache().lock());
            let sd = ffi_try!(obj_cache.struct_data
                .get_mut(&sd_h)
                .ok_or(FfiError::InvalidStructDataHandle));

            if sd.get_type_tag() != ::VERSIONED_STRUCT_DATA_TYPE_TAG {
                ffi_try!(Err(FfiError::InvalidStructuredDataTypeTag));
            }

            ffi_try!(versioned::get_all_versions(client.clone(), sd))
        };

        if n >= versions.len() {
            ffi_try!(Err(FfiError::InvalidVersionNumber));
        }

        // TODO Try to combine this code with above (extract_data) if it makes it smaller
        let immut_data_final_name = versions.remove(n);
        let resp_getter = ffi_try!(unwrap!(client.lock())
            .get(DataIdentifier::Immutable(immut_data_final_name), None));
        let immut_data_final = match ffi_try!(resp_getter.get()) {
            Data::Immutable(immut_data) => immut_data,
            _ => ffi_try!(Err(CoreError::ReceivedUnexpectedData)),
        };

        let ser_immut_data = ffi_try!(CipherOpt::decrypt(&app, immut_data_final.value()));
        let immut_data = ffi_try!(deserialise::<ImmutableData>(&ser_immut_data)
            .map_err(FfiError::from));
        let mut plain_text =
            ffi_try!(immut_data_operations::get_data(client, *immut_data.name(), None));

        *o_data = plain_text.as_mut_ptr();
        ptr::write(o_size, plain_text.len());
        ptr::write(o_capacity, plain_text.capacity());
        mem::forget(plain_text);

        0
    })
}

/// PUT StructuredData
#[no_mangle]
pub unsafe extern "C" fn struct_data_put(app: *const App, sd_h: StructDataHandle) -> i32 {
    helper::catch_unwind_i32(|| {
        let sd = ffi_try!(unwrap!(object_cache().lock())
                .struct_data
                .get_mut(&sd_h)
                .ok_or(FfiError::InvalidStructDataHandle))
            .clone();
        let data = Data::Structured(sd);
        let client = (*app).get_client();
        let resp_getter = ffi_try!(unwrap!(client.lock()).put(data, None));
        ffi_try!(resp_getter.get());

        0
    })
}

/// POST StructuredData. This will bump version.
#[no_mangle]
pub unsafe extern "C" fn struct_data_post(app: *const App, sd_h: StructDataHandle) -> i32 {
    helper::catch_unwind_i32(|| {
        let sd = ffi_try!(unwrap!(object_cache().lock())
            .struct_data
            .remove(&sd_h)
            .ok_or(FfiError::InvalidStructDataHandle));
        match struct_data_post_impl((*app).get_client(), sd.clone()) {
            Ok(new_sd) => {
                let _ = unwrap!(object_cache().lock()).struct_data.insert(sd_h, new_sd);
                0
            }
            Err(e) => {
                let _ = unwrap!(object_cache().lock()).struct_data.insert(sd_h, sd);
                ffi_try!(Err(e))
            }
        }
    })
}

fn struct_data_post_impl(client: Arc<Mutex<Client>>,
                         mut sd: StructuredData)
                         -> Result<StructuredData, FfiError> {
    let sign_key = try!(unwrap!(client.lock()).get_secret_signing_key()).clone();
    // TODO Ask routing to remove this inefficiency of requiring to clone data and all
    sd = try!(StructuredData::new(sd.get_type_tag(),
                                  *sd.name(),
                                  sd.get_version() + 1,
                                  sd.get_data().clone(),
                                  sd.get_owner_keys().clone(),
                                  sd.get_previous_owner_keys().clone(),
                                  Some(&sign_key))
        .map_err(CoreError::from));

    let data = Data::Structured(sd.clone());
    let resp_getter = try!(unwrap!(client.lock()).post(data, None));
    try!(resp_getter.get());

    Ok(sd)
}

/// DELETE StructuredData. Version will be bumped.
#[no_mangle]
pub unsafe extern "C" fn struct_data_delete(app: *const App, sd_h: StructDataHandle) -> i32 {
    helper::catch_unwind_i32(|| {
        let sd = ffi_try!(unwrap!(object_cache().lock())
            .struct_data
            .remove(&sd_h)
            .ok_or(FfiError::InvalidStructDataHandle));
        match struct_data_delete_impl((*app).get_client(), sd.clone()) {
            Ok(new_sd) => {
                let _ = unwrap!(object_cache().lock()).struct_data.insert(sd_h, new_sd);
                0
            }
            Err(e) => {
                let _ = unwrap!(object_cache().lock()).struct_data.insert(sd_h, sd);
                ffi_try!(Err(e))
            }
        }
    })
}

fn struct_data_delete_impl(client: Arc<Mutex<Client>>,
                           mut sd: StructuredData)
                           -> Result<StructuredData, FfiError> {
    let sign_key = try!(unwrap!(client.lock()).get_secret_signing_key()).clone();
    // TODO Ask routing to remove this inefficiency of requiring to clone data and all
    sd = try!(StructuredData::new(sd.get_type_tag(),
                                  *sd.name(),
                                  sd.get_version() + 1,
                                  Vec::new(),
                                  sd.get_owner_keys().clone(),
                                  sd.get_previous_owner_keys().clone(),
                                  Some(&sign_key))
        .map_err(CoreError::from));

    let data = Data::Structured(sd.clone());
    let resp_getter = try!(unwrap!(client.lock()).delete(data, None));
    try!(resp_getter.get());

    Ok(sd)
}

/// Free StructuredData handle
#[no_mangle]
pub extern "C" fn struct_data_free(handle: StructDataHandle) -> i32 {
    helper::catch_unwind_i32(|| {
        let _ = ffi_try!(unwrap!(object_cache().lock())
            .struct_data
            .remove(&handle)
            .ok_or(FfiError::InvalidStructDataHandle));

        0
    })
}

#[cfg(test)]
mod tests {
    use core::utility;
    use ffi::errors::FfiError;
    use ffi::low_level_api::{CipherOptHandle, DataIdHandle, StructDataHandle};
    use ffi::low_level_api::cipher_opt::*;
    use ffi::low_level_api::object_cache::object_cache;
    use ffi::test_utils;
    use rand;
    use std::ptr;
    use super::*;

    #[test]
    fn unversioned_struct_data_crud() {
        let app = test_utils::create_app(false);

        let mut cipher_opt_h: CipherOptHandle = 0;
        let mut sd_h: StructDataHandle = 0;
        let mut data_id_h: DataIdHandle = 0;
        let id = rand::random();
        let mut plain_text = unwrap!(utility::generate_random_vector::<u8>(10));
        unsafe {
            assert_eq!(cipher_opt_new_symmetric(&mut cipher_opt_h), 0);

            // Create
            assert_eq!(struct_data_new(&app,
                                       ::UNVERSIONED_STRUCT_DATA_TYPE_TAG,
                                       &id,
                                       cipher_opt_h,
                                       plain_text.as_ptr(),
                                       plain_text.len(),
                                       &mut sd_h),
                       0);
            assert_eq!(struct_data_extract_data_id(sd_h, &mut data_id_h), 0);

            // Put
            assert_eq!(struct_data_put(&app, sd_h), 0);
            assert!(unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));
            assert_eq!(struct_data_free(sd_h), 0);
            assert!(!unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));

            // Fetch
            assert_eq!(struct_data_fetch(&app, data_id_h, &mut sd_h), 0);
            assert!(unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));

            // Extract Data
            let rx_plain_text_0 = {
                let mut data_ptr: *mut u8 = ptr::null_mut();
                let mut data_size = 0;
                let mut capacity = 0;
                assert_eq!(struct_data_extract_data(&app,
                                                    sd_h,
                                                    &mut data_ptr,
                                                    &mut data_size,
                                                    &mut capacity),
                           0);
                Vec::from_raw_parts(data_ptr, data_size, capacity)
            };
            assert_eq!(rx_plain_text_0, plain_text);

            // New Data
            plain_text = unwrap!(utility::generate_random_vector::<u8>(10));
            assert_eq!(struct_data_new_data(&app,
                                            sd_h,
                                            cipher_opt_h,
                                            plain_text.as_ptr(),
                                            plain_text.len()),
                       0);

            // Post
            assert_eq!(struct_data_post(&app, sd_h), 0);
            assert!(unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));
            assert_eq!(struct_data_free(sd_h), 0);
            assert!(!unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));

            // Fetch
            assert_eq!(struct_data_fetch(&app, data_id_h, &mut sd_h), 0);
            assert!(unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));

            // Extract Data
            let rx_plain_text_1 = {
                let mut data_ptr: *mut u8 = ptr::null_mut();
                let mut data_size = 0;
                let mut capacity = 0;
                assert_eq!(struct_data_extract_data(&app,
                                                    sd_h,
                                                    &mut data_ptr,
                                                    &mut data_size,
                                                    &mut capacity),
                           0);
                Vec::from_raw_parts(data_ptr, data_size, capacity)
            };
            assert_eq!(rx_plain_text_1, plain_text);
            assert!(rx_plain_text_1 != rx_plain_text_0);


            // Perform Invalid Operations - should error out
            let mut versions = 0;
            assert_eq!(struct_data_num_of_versions(&app, sd_h, &mut versions),
                       FfiError::InvalidStructuredDataTypeTag.into());
            {
                let mut data_ptr: *mut u8 = ptr::null_mut();
                let mut data_size = 0;
                let mut capacity = 0;
                assert_eq!(struct_data_nth_version(&app,
                                                   sd_h,
                                                   0,
                                                   &mut data_ptr,
                                                   &mut data_size,
                                                   &mut capacity),
                           FfiError::InvalidStructuredDataTypeTag.into());
            }

            // Delete
            assert_eq!(struct_data_delete(&app, sd_h), 0);
            assert!(unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));
            assert_eq!(struct_data_free(sd_h), 0);
            assert!(!unwrap!(object_cache().lock()).struct_data.contains_key(&sd_h));

            // Fetch - should error out
            assert_eq!(struct_data_fetch(&app, data_id_h, &mut sd_h), -18);
            assert_eq!(struct_data_free(sd_h),
                       FfiError::InvalidStructDataHandle.into());
        }
    }
}
