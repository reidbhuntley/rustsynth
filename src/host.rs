use std::any::Any;

use arr_macro::arr;
use rodio::Source;

use crate::{constants::*, midi::MidiEvents, output::AudioOutput, output::AudioOutputModule};

use self::private::{
    BufferInPort, FastHashMap, ModuleBufferInHandle, ModuleBufferOutHandle,
    ModuleBuffersDescriptor, ModuleBuffersInInternal, ModuleBuffersOutInternal, ModuleHandle,
};

pub trait BufferElem: 'static + private::BufferElemSealed + Default + Clone {
    fn new_buffer(self) -> Buffer<Self> {
        arr![self.clone(); 512]
    }

    fn new_vec<T: BufferElem>(len: usize) -> Vec<T> {
        std::iter::repeat(T::default()).take(len).collect()
    }
}

impl BufferElem for f32 {}

impl BufferElem for MidiEvents {}

mod private {
    use std::{collections::HashMap, sync::atomic::AtomicUsize};

    use seahash::SeaHasher;

    use super::{
        BufferElem, BufferHandle, BufferInHandle, BufferOutHandle, BuffersInExt, BuffersOutExt,
        ModuleBuffersIn, ModuleBuffersOut, ModuleDescriptor,
    };

    #[derive(Clone, Default)]
    pub struct BuildHasher;

    impl std::hash::BuildHasher for BuildHasher {
        type Hasher = SeaHasher;

        #[inline(always)]
        fn build_hasher(&self) -> Self::Hasher {
            SeaHasher::new()
        }
    }

    pub type FastHashMap<K, V> = HashMap<K, V, BuildHasher>;

    #[derive(Clone, Copy, Default, PartialEq, Eq)]
    pub struct ModuleHandle {
        pub idx: usize,
    }

    #[derive(Educe, Eq)]
    #[educe(Clone, Copy, PartialEq)]
    pub struct ModuleBufferInHandle<T: BufferElem> {
        pub module_handle: ModuleHandle,
        pub buf_handle: BufferInHandle<T>,
    }

    #[derive(Educe, Eq)]
    #[educe(Clone, Copy, PartialEq)]
    pub struct ModuleBufferOutHandle<T: BufferElem> {
        pub module_handle: ModuleHandle,
        pub buf_handle: BufferOutHandle<T>,
    }

    pub struct BufferOutPort<T: BufferElem> {
        pub buffer: super::Buffer<T>,
        pub dependents: Vec<ModuleBufferInHandle<T>>,
    }

    pub enum BufferInPort<T: BufferElem> {
        OutBuffer(ModuleBufferOutHandle<T>),
        Constant(super::Buffer<T>),
    }

    impl<T: BufferElem> BufferInPort<T> {
        pub fn with_constant(value: T) -> Self {
            Self::Constant(T::new_buffer(value))
        }
    }

    impl<T: BufferElem> Default for BufferInPort<T> {
        fn default() -> Self {
            Self::with_constant(T::default())
        }
    }

    #[derive(Default)]
    pub struct BufferInPorts<T: BufferElem> {
        pub buffers: Vec<BufferInPort<T>>,
        pub handles: FastHashMap<String, BufferInHandle<T>>,
    }

    impl<T: BufferElem> BufferInPorts<T> {
        fn add_buffer(&mut self, buffer: BufferInPort<T>, name: &str) -> BufferInHandle<T> {
            if !self.handles.contains_key(name) {
                self.buffers.push(buffer);
                let handle = BufferInHandle(BufferHandle::new(self.buffers.len() - 1));
                self.handles.insert(name.to_owned(), handle);
                handle
            } else {
                panic!()
            }
        }
    }

    #[derive(Default)]
    pub struct BufferOutPorts<T: BufferElem> {
        pub buffers: Vec<BufferOutPort<T>>,
        pub handles: FastHashMap<String, BufferOutHandle<T>>,
    }

    impl<T: BufferElem> BufferOutPorts<T> {
        fn add_buffer(&mut self, buffer: BufferOutPort<T>, name: &str) -> BufferOutHandle<T> {
            if !self.handles.contains_key(name) {
                self.buffers.push(buffer);
                let handle = BufferOutHandle(BufferHandle::new(self.buffers.len() - 1));
                self.handles.insert(name.to_owned(), handle);
                handle
            } else {
                panic!()
            }
        }
    }

    #[derive(Default, Clone)]
    pub struct ModuleBuffersDescriptor<T: BufferElem> {
        pub buf_in: Vec<(String, T)>,
        pub buf_out: Vec<String>,
    }

    #[derive(Default)]
    pub struct ModuleBuffersInInternal {
        pub num_dependencies: usize,
        pub num_finished_dependencies: AtomicUsize,
        buf_signal: BufferInPorts<f32>,
        buf_midi: BufferInPorts<crate::midi::MidiEvents>,
    }

    impl ModuleBuffersInInternal {
        pub fn new(descriptors: &ModuleDescriptor) -> Self {
            let mut out = Self::default();
            out.add_num_buffers_all(&descriptors);
            out
        }

        pub fn add_num_buffers_all(&mut self, descriptors: &ModuleDescriptor) {
            Self::add_num_buffers(&mut self.buf_signal, &descriptors.buf_signal);
            Self::add_num_buffers(&mut self.buf_midi, &descriptors.buf_midi);
        }

        pub fn add_num_buffers<T: BufferElem>(
            buffers: &mut BufferInPorts<T>,
            defaults: &ModuleBuffersDescriptor<T>,
        ) {
            for (name, default) in defaults.buf_in.iter() {
                buffers.add_buffer(BufferInPort::with_constant(default.clone()), name);
            }
        }
    }

    #[derive(Default)]
    pub struct ModuleBuffersOutInternal {
        buf_signal: BufferOutPorts<f32>,
        buf_midi: BufferOutPorts<crate::midi::MidiEvents>,
    }

    impl ModuleBuffersOutInternal {
        pub fn new(descriptors: &ModuleDescriptor) -> Self {
            let mut out = Self::default();
            out.add_num_buffers_all(&descriptors);
            out
        }

        pub fn add_num_buffers_all(&mut self, descriptors: &ModuleDescriptor) {
            Self::add_num_buffers(&mut self.buf_signal, &descriptors.buf_signal);
            Self::add_num_buffers(&mut self.buf_midi, &descriptors.buf_midi);
        }

        pub fn add_num_buffers<T: BufferElem>(
            buffers: &mut BufferOutPorts<T>,
            descriptor: &ModuleBuffersDescriptor<T>,
        ) {
            for name in descriptor.buf_out.iter() {
                buffers.add_buffer(
                    BufferOutPort {
                        buffer: T::new_buffer(T::default()),
                        dependents: Vec::new(),
                    },
                    name,
                );
            }
        }
    }

    pub trait BufferElemSealed {
        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferInPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferInPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferOutPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_buffers_out_mut(buf_out: &mut ModuleBuffersOutInternal) -> &mut BufferOutPorts<Self>
        where
            Self: Sized + BufferElem;

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self>
        where
            Self: Sized + BufferElem;

        fn get_descriptor(descriptors: &mut ModuleDescriptor) -> &mut ModuleBuffersDescriptor<Self>
        where
            Self: Sized + BufferElem;
    }
    impl BufferElemSealed for f32 {
        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferInPorts<Self> {
            &buf_in.buf_signal
        }

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferInPorts<Self> {
            &mut buf_in.buf_signal
        }

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self> {
            &buf_in.buf_signal
        }

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferOutPorts<Self> {
            &buf_out.buf_signal
        }

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferOutPorts<Self> {
            &mut buf_out.buf_signal
        }

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self> {
            &buf_out.buf_signal
        }

        fn get_descriptor(
            descriptors: &mut ModuleDescriptor,
        ) -> &mut ModuleBuffersDescriptor<Self> {
            &mut descriptors.buf_signal
        }
    }
    impl BufferElemSealed for crate::midi::MidiEvents {
        fn get_buffers_in(buf_in: &ModuleBuffersInInternal) -> &BufferInPorts<Self> {
            &buf_in.buf_midi
        }

        fn get_buffers_in_mut(buf_in: &mut ModuleBuffersInInternal) -> &mut BufferInPorts<Self> {
            &mut buf_in.buf_midi
        }

        fn get_ext_buffers_in(buf_in: &ModuleBuffersIn) -> &BuffersInExt<Self> {
            &buf_in.buf_midi
        }

        fn get_buffers_out(buf_out: &ModuleBuffersOutInternal) -> &BufferOutPorts<Self> {
            &buf_out.buf_midi
        }

        fn get_buffers_out_mut(
            buf_out: &mut ModuleBuffersOutInternal,
        ) -> &mut BufferOutPorts<Self> {
            &mut buf_out.buf_midi
        }

        fn get_ext_buffers_out(buf_out: &ModuleBuffersOut) -> &BuffersOutExt<Self> {
            &buf_out.buf_midi
        }

        fn get_descriptor(
            descriptors: &mut ModuleDescriptor,
        ) -> &mut ModuleBuffersDescriptor<Self> {
            &mut descriptors.buf_midi
        }
    }
}

pub type Buffer<T> = [T; BUFFER_LEN];

type BufferHandleRaw = usize;

#[derive(Clone, Eq)]
struct BufferHandle<T: BufferElem> {
    _marker: std::marker::PhantomData<T>,
    idx: BufferHandleRaw,
}

impl<T: BufferElem> PartialEq for BufferHandle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}

impl<T: BufferElem> Copy for BufferHandle<T> {}

impl<T: BufferElem> BufferHandle<T> {
    fn new(idx: BufferHandleRaw) -> Self {
        Self {
            idx,
            _marker: Default::default(),
        }
    }
}

#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct BufferInHandle<T: BufferElem>(BufferHandle<T>);
#[derive(Educe, Eq)]
#[educe(Clone, Copy, PartialEq)]
pub struct BufferOutHandle<T: BufferElem>(BufferHandle<T>);

#[derive(Default, Clone)]
pub struct ModuleDescriptor {
    buf_signal: ModuleBuffersDescriptor<f32>,
    buf_midi: ModuleBuffersDescriptor<MidiEvents>,
}

pub struct BuiltModuleDescriptor<T: Module> {
    initial_data: Box<T>,
    buffers_descriptors: ModuleDescriptor,
}

impl ModuleDescriptor {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn build<T: Module>(self, initial_data: T) -> BuiltModuleDescriptor<T> {
        BuiltModuleDescriptor {
            initial_data: Box::new(initial_data),
            buffers_descriptors: self,
        }
    }

    pub fn with_buf_in_default<E: BufferElem>(
        &mut self,
        name: &str,
        default: E,
    ) -> BufferInHandle<E> {
        let buffers_in = &mut E::get_descriptor(self).buf_in;
        buffers_in.push((name.to_owned(), default));
        BufferInHandle(BufferHandle::new(buffers_in.len() - 1))
    }

    pub fn with_buf_in<E: BufferElem>(&mut self, name: &str) -> BufferInHandle<E> {
        self.with_buf_in_default::<E>(name, Default::default())
    }

    pub fn with_buf_out<E: BufferElem>(&mut self, name: &str) -> BufferOutHandle<E> {
        let buffers_out = &mut E::get_descriptor(self).buf_out;
        buffers_out.push(name.to_owned());
        BufferOutHandle(BufferHandle::new(buffers_out.len() - 1))
    }
}

type BuffersInExt<T> = Vec<*const Buffer<T>>;

pub struct ModuleBuffersIn {
    buf_signal: BuffersInExt<f32>,
    buf_midi: BuffersInExt<MidiEvents>,
}

impl ModuleBuffersIn {
    pub fn get<T: BufferElem>(&self, buffer: BufferInHandle<T>) -> &Buffer<T> {
        let bufs = T::get_ext_buffers_in(&self)[buffer.0.idx];
        unsafe { &*bufs }
    }

    pub fn iter<T: BufferElem>(&self) -> impl Iterator<Item = &Buffer<T>> + '_ {
        T::get_ext_buffers_in(&self)
            .iter()
            .map(|&bufs| unsafe { &*bufs })
    }
}

type BuffersOutExt<T> = Vec<*mut Buffer<T>>;

pub struct ModuleBuffersOut {
    buf_signal: Vec<*mut Buffer<f32>>,
    buf_midi: Vec<*mut Buffer<MidiEvents>>,
}

impl ModuleBuffersOut {
    pub fn get<T: BufferElem>(&mut self, buffer: BufferOutHandle<T>) -> &mut Buffer<T> {
        let bufs = T::get_ext_buffers_out(&self)[buffer.0.idx];
        unsafe { &mut *bufs }
    }

    pub fn iter_mut<T: BufferElem>(&mut self) -> impl Iterator<Item = &mut Buffer<T>> + '_ {
        T::get_ext_buffers_out(self)
            .iter()
            .map(|&bufs| unsafe { &mut *bufs })
    }
}

pub trait ModuleTypes {
    type Settings;
}

pub trait Module: 'static + Any {
    fn init(settings: Self::Settings) -> BuiltModuleDescriptor<Self>
    where
        Self: Sized + ModuleTypes;
    fn fill_buffers(&mut self, buffers_in: &ModuleBuffersIn, buffers_out: &mut ModuleBuffersOut);
}

struct ModuleInternals {
    module: Box<dyn Module>,
    buf_in: ModuleBuffersInInternal,
    buf_out: ModuleBuffersOutInternal,
}

impl ModuleInternals {
    fn new<T: Module + ModuleTypes>(settings: T::Settings) -> Self {
        let descriptor = T::init(settings);
        Self {
            module: descriptor.initial_data,
            buf_in: ModuleBuffersInInternal::new(&descriptor.buffers_descriptors),
            buf_out: ModuleBuffersOutInternal::new(&descriptor.buffers_descriptors),
        }
    }
}

pub struct Host {
    modules: FastHashMap<usize, ModuleInternals>,
    handles: FastHashMap<String, ModuleHandle>,
    next_idx: usize,
    output: rodio::source::Stoppable<AudioOutput>,
}

const OUTPUT_MODULE_NAME: &str = "audio_out";

impl Host {
    pub fn new() -> Self {
        let output = AudioOutput::new();
        let mut out = Self {
            modules: Default::default(),
            handles: Default::default(),
            next_idx: 0,
            output: output.clone().stoppable(),
        };
        out.create_module::<AudioOutputModule>(OUTPUT_MODULE_NAME, output);
        out
    }

    pub fn create_module<T: Module + ModuleTypes>(&mut self, name: &str, settings: T::Settings) {
        if self.handles.contains_key(name) {
            panic!()
        }
        let module = ModuleInternals::new::<T>(settings);
        let idx = self.next_idx;
        self.next_idx += 1;
        self.modules.insert(idx, module);
        self.handles.insert(name.to_owned(), ModuleHandle { idx });
    }

    fn set_buffer_in<T: BufferElem>(
        &mut self,
        port_handle: ModuleBufferInHandle<T>,
        new: BufferInPort<T>,
    ) {
        let module_in = self
            .modules
            .get_mut(&port_handle.module_handle.idx)
            .unwrap();

        let port = &T::get_buffers_in(&module_in.buf_in).buffers[port_handle.buf_handle.0.idx];
        let (old_out, new_out) = match port {
            BufferInPort::OutBuffer(out) => (
                Some(out.clone()),
                match &new {
                    BufferInPort::Constant(_) => {
                        module_in.buf_in.num_dependencies -= 1;
                        None
                    }
                    BufferInPort::OutBuffer(new_out) => Some(new_out.clone()),
                },
            ),
            BufferInPort::Constant(_) => (
                None,
                match &new {
                    BufferInPort::Constant(_) => None,
                    BufferInPort::OutBuffer(new_out) => {
                        module_in.buf_in.num_dependencies += 1;
                        Some(new_out.clone())
                    }
                },
            ),
        };

        T::get_buffers_in_mut(&mut module_in.buf_in).buffers[port_handle.buf_handle.0.idx] = new;

        if let Some(old_out) = old_out {
            let module_out = self.modules.get_mut(&old_out.module_handle.idx).unwrap();
            T::get_buffers_out_mut(&mut module_out.buf_out).buffers[old_out.buf_handle.0.idx]
                .dependents
                .retain(|d| d != &port_handle);
        }

        if let Some(new_out) = new_out {
            let module_out = self.modules.get_mut(&new_out.module_handle.idx).unwrap();
            T::get_buffers_out_mut(&mut module_out.buf_out).buffers[new_out.buf_handle.0.idx]
                .dependents
                .push(port_handle);
        }
    }

    pub fn link<T: BufferElem>(
        &mut self,
        module_out: &str,
        buf_name_out: &str,
        module_in: &str,
        buf_name_in: &str,
    ) {
        let handle_out = self.handles[module_out];
        let buf_out = ModuleBufferOutHandle {
            module_handle: handle_out,
            buf_handle: T::get_buffers_out(&self.modules[&handle_out.idx].buf_out).handles
                [buf_name_out],
        };

        let handle_in = self.handles[module_in];
        let buf_in = ModuleBufferInHandle {
            module_handle: handle_in,
            buf_handle: T::get_buffers_in(&self.modules[&handle_in.idx].buf_in).handles
                [buf_name_in],
        };

        self.set_buffer_in(buf_in, BufferInPort::OutBuffer(buf_out));
    }

    pub fn link_value<T: BufferElem>(&mut self, value: T, module_in: &str, buf_name_in: &str) {
        let handle_in = self.handles[module_in];
        let buf_in = ModuleBufferInHandle {
            module_handle: handle_in,
            buf_handle: T::get_buffers_in(&self.modules[&handle_in.idx].buf_in).handles
                [buf_name_in],
        };

        self.set_buffer_in(buf_in, BufferInPort::with_constant(value));
    }

    pub fn destroy_module(&mut self, module: &str) {
        fn remove_dependents<T: BufferElem>(host: &mut Host, module: &ModuleInternals) {
            for out_port in T::get_buffers_out(&module.buf_out).buffers.iter() {
                for dep_handle in out_port.dependents.iter() {
                    host.set_buffer_in(dep_handle.clone(), BufferInPort::default());
                }
            }
        }

        fn remove_dependencies<T: BufferElem>(
            host: &mut Host,
            module: &ModuleInternals,
            module_handle: ModuleHandle,
        ) {
            for (port_idx, in_port) in T::get_buffers_in(&module.buf_in).buffers.iter().enumerate()
            {
                match in_port {
                    BufferInPort::OutBuffer(_) => {
                        host.set_buffer_in(
                            ModuleBufferInHandle {
                                module_handle: module_handle.clone(),
                                buf_handle: BufferInHandle(BufferHandle::new(port_idx)),
                            },
                            BufferInPort::<T>::default(),
                        );
                    }
                    BufferInPort::Constant(_) => {}
                }
            }
        }

        if module == OUTPUT_MODULE_NAME {
            panic!()
        }

        let handle = self.handles.remove(module).unwrap();

        let module = self.modules.remove(&handle.idx).unwrap();
        remove_dependents::<f32>(self, &module);
        remove_dependencies::<f32>(self, &module, handle);
        remove_dependents::<MidiEvents>(self, &module);
        remove_dependencies::<MidiEvents>(self, &module, handle);
    }

    // pub fn update_module<T: Module + ModuleTypes>(
    //     &mut self,
    //     handle: &ModuleHandle,
    //     settings: T::Settings,
    // ) {
    //     if let Some(module) = self.modules.get_mut(&handle.idx) {
    //         assert!(module.module.as_ref().type_id() == TypeId::of::<T>());
    //         let descriptor = T::init(settings);
    //         module.module = descriptor.initial_data;
    //         // module.buf_in.set_num_buffers_all(&descriptor.buf_in);
    //         // module.buf_out.set_num_buffers_all(&descriptor.buf_out);
    //     }
    // }

    pub fn process(&mut self) -> ! {
        let (_stream, stream_handle) = rodio::OutputStream::try_default().unwrap();
        stream_handle
            .play_raw(self.output.clone().stoppable())
            .unwrap();

        loop {
            for module in self.modules.values_mut() {
                *module.buf_in.num_finished_dependencies.get_mut() = 0;
            }

            let zero_dependency_mods = self
                .modules
                .iter()
                .filter_map(|(&idx, module)| {
                    if module.buf_in.num_dependencies == 0 {
                        Some(ModuleHandle { idx })
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            for handle in zero_dependency_mods {
                unsafe { self.process_module(handle) };
            }
        }
    }

    unsafe fn process_module(&mut self, handle: ModuleHandle) {
        fn get_linked_ports<T: BufferElem>(
            host: &Host,
            module: &ModuleInternals,
        ) -> Vec<*const Buffer<T>> {
            T::get_buffers_in(&module.buf_in)
                .buffers
                .iter()
                .map(|port| match port {
                    BufferInPort::OutBuffer(handle) => {
                        let buf_out = &host.modules.get(&handle.module_handle.idx).unwrap().buf_out;
                        &T::get_buffers_out(buf_out).buffers[handle.buf_handle.0.idx].buffer
                            as *const _
                    }
                    BufferInPort::Constant(buf) => buf,
                })
                .collect::<Vec<_>>()
        }

        fn get_out_buffers<T: BufferElem>(module: &mut ModuleInternals) -> Vec<*mut Buffer<T>> {
            T::get_buffers_out_mut(&mut module.buf_out)
                .buffers
                .iter_mut()
                .map(|buf| &mut buf.buffer as *mut _)
                .collect()
        }

        fn get_dependents<T: BufferElem>(
            module: &ModuleInternals,
        ) -> impl Iterator<Item = ModuleHandle> + '_ {
            T::get_buffers_out(&module.buf_out)
                .buffers
                .iter()
                .flat_map(|buf| &buf.dependents)
                .map(|h| h.module_handle.clone())
        }

        let module = self.modules.get_mut(&handle.idx).unwrap() as *mut ModuleInternals;

        let module_ref = &*module;

        if module_ref
            .buf_in
            .num_finished_dependencies
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1
            < module_ref.buf_in.num_dependencies
        {
            return;
        }

        let buf_in = ModuleBuffersIn {
            buf_signal: get_linked_ports(&self, module_ref),
            buf_midi: get_linked_ports(&self, module_ref),
        };

        let module_mut = &mut *module;
        let mut buf_out = ModuleBuffersOut {
            buf_signal: get_out_buffers(module_mut),
            buf_midi: get_out_buffers(module_mut),
        };
        module_mut.module.fill_buffers(&buf_in, &mut buf_out);

        for dependent in
            get_dependents::<f32>(module_mut).chain(get_dependents::<MidiEvents>(module_mut))
        {
            self.process_module(dependent);
        }
    }
}
